/**
 * Lecture locale de `HX_ModelCatalog.json` pour `presetMeta` (canal, signal, etc.).
 */

export type PresetMetaJson = {
  categoryId?: number;
  categoryName?: string;
  chainHex?: string | string[];
  channel?: string;
  signal?: string | string[];
  emulationName?: string;
};

type CatalogModelEntry = {
  id: string | null;
  presetMeta: PresetMetaJson | null;
  /** Fichier PNG sous `icons_models/` (ex. `FX_HX_EQ_SimpleTilt.png`). */
  image: string | null;
  /** Ordre des clés `params` côté catalogue (`HX_ModelCatalog.json`). */
  catalogParamOrder: string[];
};

type CatalogIndexes = {
  /** Vue historique : clé = catégorie + nom. */
  byCategoryAndName: Map<string, CatalogModelEntry>;
  /** Jointure stricte du slot preset : `chainHex` -> entrée catalogue. */
  byHex: Map<string, CatalogModelEntry>;
  /** Jointure stricte par ID catalogue -> entrée. */
  byId: Map<string, CatalogModelEntry>;
};

let catalogIndexesPromise: Promise<CatalogIndexes> | null = null;

function catalogKey(category: string, modelName: string): string {
  return `${category.trim().toLowerCase()}\0${modelName.trim().toLowerCase()}`;
}

function normalizeHexList(chainHex: string | string[] | undefined): string[] {
  if (chainHex === undefined || chainHex === null) return [];
  if (typeof chainHex === "string") {
    const t = chainHex.trim().toLowerCase();
    return t ? [t] : [];
  }
  return chainHex.map((h) => String(h).trim().toLowerCase()).filter(Boolean);
}

function extractCatalogParamOrder(paramsRaw: unknown): string[] {
  const out: string[] = [];
  const walk = (v: unknown) => {
    if (Array.isArray(v)) {
      for (const x of v) walk(x);
      return;
    }
    if (!v || typeof v !== "object") return;
    for (const k of Object.keys(v as Record<string, unknown>)) {
      const kk = k.trim();
      if (kk) out.push(kk);
    }
  };
  walk(paramsRaw);
  return out;
}

/**
 * Hex lu sur le preset pour un slot Amp+Cab : `<ampHex>1a<cabHex>` (cf. Rust `cab_info_from_module_id`).
 * Le catalogue aligne souvent `chainHex` sur l’ampli seul — on retente donc la partie amp après l’échec du match complet.
 */
function moduleHexCatalogLookupCandidates(hexNorm: string): string[] {
  const out: string[] = [hexNorm];
  const sep = "1a";
  const i = hexNorm.indexOf(sep);
  if (i > 0) {
    const ampPart = hexNorm.slice(0, i);
    if (ampPart.length > 0) out.push(ampPart);
  }
  return out;
}

async function loadCatalogModelIndexes(): Promise<CatalogIndexes> {
  const url = "/src-tauri/resources/HX_ModelCatalog.json";
  const res = await fetch(url);
  if (!res.ok) {
    console.warn("HX_ModelCatalog.json : chargement presetMeta impossible.", res.status);
    return { byCategoryAndName: new Map(), byHex: new Map(), byId: new Map() };
  }
  const raw = await res.text();
  const data = JSON.parse(raw) as { models?: unknown[]; categories?: unknown[] };
  const byCategoryAndName = new Map<string, CatalogModelEntry>();
  const byHex = new Map<string, CatalogModelEntry>();
  const byId = new Map<string, CatalogModelEntry>();
  const record = (catName: string, models: unknown) => {
    if (!Array.isArray(models)) return;
    for (const m of models) {
      if (!m || typeof m !== "object") continue;
      const mo = m as {
        id?: string | number;
        name?: string;
        presetMeta?: PresetMetaJson;
        image?: string;
        params?: unknown;
      };
      const name = typeof mo.name === "string" ? mo.name.trim() : "";
      if (!name) continue;
      const key = catalogKey(catName, name);
      const idRaw = mo.id;
      const id =
        typeof idRaw === "string"
          ? (idRaw.trim() || null)
          : typeof idRaw === "number"
            ? String(idRaw)
            : null;
      const imgRaw = mo.image;
      const image =
        typeof imgRaw === "string" && imgRaw.trim().length > 0 ? imgRaw.trim() : null;
      const entry: CatalogModelEntry = {
        id,
        presetMeta: mo.presetMeta ? { ...mo.presetMeta } : null,
        image,
        catalogParamOrder: extractCatalogParamOrder(mo.params),
      };
      // Pour compat historique: garder la première entrée d'un (category, name).
      if (!byCategoryAndName.has(key)) {
        byCategoryAndName.set(key, entry);
      }
      if (id && !byId.has(id)) {
        byId.set(id, entry);
      }
      for (const h of normalizeHexList(entry.presetMeta?.chainHex)) {
        if (!byHex.has(h)) byHex.set(h, entry);
      }
    }
  };
  if (Array.isArray(data.models) && data.models.length > 0) {
    for (const m of data.models) {
      if (!m || typeof m !== "object") continue;
      const mo = m as {
        id?: string | number;
        name?: string;
        presetMeta?: PresetMetaJson;
        image?: string;
        params?: unknown;
      };
      const cn = mo.presetMeta?.categoryName;
      const catName =
        typeof cn === "string" && cn.trim().length > 0 ? cn.trim() : "Unknown";
      record(catName, [m]);
    }
    return { byCategoryAndName, byHex, byId };
  }
  for (const cat of data.categories ?? []) {
    if (!cat || typeof cat !== "object") continue;
    const c = cat as { name?: string; models?: unknown; subcategories?: unknown[] };
    const cn = typeof c.name === "string" ? c.name : "";
    if (!cn) continue;
    record(cn, c.models);
    for (const sub of c.subcategories ?? []) {
      if (!sub || typeof sub !== "object") continue;
      record(cn, (sub as { models?: unknown }).models);
    }
  }
  return { byCategoryAndName, byHex, byId };
}

async function getCatalogIndexes(): Promise<CatalogIndexes> {
  if (!catalogIndexesPromise) {
    catalogIndexesPromise = loadCatalogModelIndexes().catch((e) => {
      catalogIndexesPromise = null;
      throw e;
    });
  }
  return catalogIndexesPromise;
}

export async function getPresetMetaForModel(
  slotCategory: string,
  modelDisplayName: string,
): Promise<PresetMetaJson | null> {
  const idx = await getCatalogIndexes();
  return idx.byCategoryAndName.get(catalogKey(slotCategory, modelDisplayName))?.presetMeta ?? null;
}

export async function getCatalogModelIdForModel(
  slotCategory: string,
  modelDisplayName: string,
): Promise<string | null> {
  const idx = await getCatalogIndexes();
  return idx.byCategoryAndName.get(catalogKey(slotCategory, modelDisplayName))?.id ?? null;
}

/** Jointure stricte via `presetMeta.chainHex` (hex module lu dans le preset). */
export async function getCatalogModelIdForHex(moduleHex: string | undefined): Promise<string | null> {
  const hexNorm = (moduleHex ?? "").trim().toLowerCase();
  if (!hexNorm) return null;
  const idx = await getCatalogIndexes();
  for (const h of moduleHexCatalogLookupCandidates(hexNorm)) {
    const entry = idx.byHex.get(h);
    if (entry?.id) return entry.id;
  }
  return null;
}

/** Nom de fichier `image` du catalogue (ex. `FX_HX_EQ_SimpleTilt.png`), ou `null`. */
export async function getCatalogModelImageForModel(
  slotCategory: string,
  modelDisplayName: string,
): Promise<string | null> {
  const idx = await getCatalogIndexes();
  return idx.byCategoryAndName.get(catalogKey(slotCategory, modelDisplayName))?.image ?? null;
}

export async function getPresetMetaForId(modelId: string | null | undefined): Promise<PresetMetaJson | null> {
  const id = (modelId ?? "").trim();
  if (!id) return null;
  const idx = await getCatalogIndexes();
  return idx.byId.get(id)?.presetMeta ?? null;
}

export async function getCatalogModelImageForId(modelId: string | null | undefined): Promise<string | null> {
  const id = (modelId ?? "").trim();
  if (!id) return null;
  const idx = await getCatalogIndexes();
  return idx.byId.get(id)?.image ?? null;
}

export async function getCatalogParamOrderForId(
  modelId: string | null | undefined,
): Promise<string[] | null> {
  const id = (modelId ?? "").trim();
  if (!id) return null;
  const idx = await getCatalogIndexes();
  const order = idx.byId.get(id)?.catalogParamOrder ?? [];
  return order.length > 0 ? [...order] : null;
}

export function pickChannel(meta: PresetMetaJson | null): string | null {
  const c = meta?.channel;
  if (typeof c !== "string") return null;
  const t = c.trim();
  return t.length > 0 ? t : null;
}

export function pickEmulationName(meta: PresetMetaJson | null): string | null {
  const e = meta?.emulationName;
  if (typeof e !== "string") return null;
  const t = e.trim();
  return t.length > 0 ? t : null;
}

/**
 * `signal` : chaîne unique, ou tableau aligné sur `chainHex` (même ordre) quand `moduleHex` est connu.
 */
export function pickSignal(meta: PresetMetaJson | null, moduleHex: string | undefined): string | null {
  if (!meta) return null;
  const sig = meta.signal;
  if (sig === undefined || sig === null) return null;
  if (typeof sig === "string") {
    const t = sig.trim();
    return t.length > 0 ? t : null;
  }
  if (!Array.isArray(sig)) return null;
  const hexNorm = (moduleHex ?? "").trim().toLowerCase();
  const hexList = normalizeHexList(meta.chainHex);
  if (hexNorm && hexList.length > 0) {
    for (const h of moduleHexCatalogLookupCandidates(hexNorm)) {
      const idx = hexList.indexOf(h);
      if (idx >= 0 && idx < sig.length) {
        const s = sig[idx];
        if (typeof s === "string" && s.trim()) return s.trim();
      }
    }
  }
  for (const x of sig) {
    if (typeof x === "string" && x.trim()) return x.trim();
  }
  return null;
}

/** Tests uniquement. */
export function resetHxCatalogMetaMapForTests(): void {
  catalogIndexesPromise = null;
}
