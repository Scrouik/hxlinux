/**
 * Lecture locale de `HX_ModelCatalog.json` pour `presetMeta` (basedOn, subCategory, etc.).
 */

export type PresetMetaJson = {
  categoryId?: number;
  categoryName?: string;
  chainHex?: string | string[];
  /** Légende « Based on » (CSV Line 6 / parsing nom). */
  basedOn?: string;
  /** Sous-catégorie instrument (ex. Guitar) ou paires type Mono/Stereo depuis le CSV. */
  subCategory?: string | string[];
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
    return {
      byCategoryAndName: new Map(),
      byHex: new Map(),
      byId: new Map(),
    };
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

export function pickBasedOn(meta: PresetMetaJson | null): string | null {
  const c = meta?.basedOn;
  if (typeof c !== "string") return null;
  const t = c.trim();
  return t.length > 0 ? t : null;
}

/** Normalise une étiquette mono/stéréo pour le routage des params (minuscules). */
function normalizeRoutingMonoStereo(label: string | null | undefined): "mono" | "stereo" | null {
  const s = (label ?? "").trim().toLowerCase();
  if (!s) return null;
  if (s.includes("stereo")) return "stereo";
  if (s.includes("mono")) return "mono";
  return null;
}

/**
 * Signal de routage **mono | stéréo** pour l’UI params (masquage stereo-only, variantes .models).
 * Déduit d’abord de `chainHex` + `moduleHex` (paires mono/stéréo), puis des libellés dans `subCategory`
 * si ce sont encore des chaînes « mono » / « stereo » (ou un tableau parallèle à `chainHex`).
 */
export function pickSignal(meta: PresetMetaJson | null, moduleHex: string | undefined): string | null {
  if (!meta) return null;
  const hexList = normalizeHexList(meta.chainHex);
  const hexNorm = (moduleHex ?? "").trim().toLowerCase();

  if (hexNorm && hexList.length >= 2) {
    for (const h of moduleHexCatalogLookupCandidates(hexNorm)) {
      const idx = hexList.indexOf(h);
      if (idx >= 0) {
        return idx === 0 ? "mono" : "stereo";
      }
    }
  }

  const sc = meta.subCategory;
  if (typeof sc === "string") {
    const t = sc.trim();
    const r = normalizeRoutingMonoStereo(t);
    if (r) return r;
  }
  if (Array.isArray(sc)) {
    if (hexNorm && hexList.length > 0 && sc.length === hexList.length) {
      for (const h of moduleHexCatalogLookupCandidates(hexNorm)) {
        const idx = hexList.indexOf(h);
        if (idx >= 0 && idx < sc.length) {
          const cell = sc[idx];
          if (typeof cell === "string") {
            const r = normalizeRoutingMonoStereo(cell);
            if (r) return r;
          }
        }
      }
    }
    for (const x of sc) {
      if (typeof x === "string") {
        const r = normalizeRoutingMonoStereo(x);
        if (r) return r;
      }
    }
  }

  if (hexList.length === 1 && hexNorm) {
    for (const h of moduleHexCatalogLookupCandidates(hexNorm)) {
      if (hexList.includes(h)) return "mono";
    }
  }
  return null;
}

/** Libellé sous-catégorie pour l’en-tête, résolu sur la variante `chainHex` active si possible. */
export function formatSubCategoryForHeader(
  meta: PresetMetaJson | null,
  moduleHex: string | undefined,
): string | null {
  const sc = meta?.subCategory;
  if (sc === undefined || sc === null) return null;
  if (typeof sc === "string") {
    const t = sc.trim();
    return t.length > 0 ? t : null;
  }
  if (!Array.isArray(sc)) return null;
  const bits = sc.map((x) => (typeof x === "string" ? x.trim() : ""));
  const nonEmpty = bits.filter(Boolean);
  if (nonEmpty.length === 0) return null;

  // Cas paires/parallèles: `chainHex: [..]` et `subCategory: [..]`.
  // On choisit la valeur de même index que le `moduleHex` actif.
  const hexNorm = (moduleHex ?? "").trim().toLowerCase();
  const hexList = normalizeHexList(meta?.chainHex);
  if (hexNorm && hexList.length > 0 && bits.length === hexList.length) {
    for (const h of moduleHexCatalogLookupCandidates(hexNorm)) {
      const idx = hexList.indexOf(h);
      if (idx >= 0 && idx < bits.length) {
        const resolved = bits[idx];
        if (resolved) return resolved;
      }
    }
  }

  // Repli conservateur : garder la première valeur non vide, sans concaténer toutes les variantes.
  return nonEmpty[0] ?? null;
}

/** Tests uniquement. */
export function resetHxCatalogMetaMapForTests(): void {
  catalogIndexesPromise = null;
  catalogPickerDataPromise = null;
}

// --- Sélecteur visuel catégorie / sous-catégorie / modèle (aperçu, pas d’écriture USB) ---

export type CatalogPickerModelRow = {
  id: string;
  name: string;
};

export type CatalogPickerData = {
  /** Noms `presetMeta.categoryName` triés (hors « None »). */
  categories: string[];
  /** Sous-clés triées par catégorie (une entrée par valeur `subCategory`, tableaux explosés). */
  subcategoriesByCategory: Map<string, string[]>;
  /** Clé = `catalogPickerRowKey(cat, sub)` → modèles triés par nom (id unique). */
  modelsByCategoryAndSub: Map<string, CatalogPickerModelRow[]>;
};

let catalogPickerDataPromise: Promise<CatalogPickerData> | null = null;

/**
 * Valeurs distinctes de `presetMeta.subCategory` pour le picker :
 * chaîne seule → un élément ; tableau (ex. Mono/Stéréo, Single/Dual) → **une entrée par cellule**, pas une concaténation.
 */
export function presetMetaSubCategoryPickerKeys(
  meta: PresetMetaJson | null | undefined,
): string[] {
  const sc = meta?.subCategory;
  if (sc === undefined || sc === null) return ["—"];
  if (typeof sc === "string") {
    const t = sc.trim();
    return [t.length > 0 ? t : "—"];
  }
  if (Array.isArray(sc)) {
    const seen = new Set<string>();
    const out: string[] = [];
    for (const x of sc) {
      if (typeof x !== "string") continue;
      const t = x.trim();
      if (!t) continue;
      const low = t.toLowerCase();
      if (seen.has(low)) continue;
      seen.add(low);
      out.push(t);
    }
    return out.length > 0 ? out : ["—"];
  }
  return ["—"];
}

export function catalogPickerRowKey(category: string, subKey: string): string {
  return `${category.trim()}\0${subKey}`;
}

async function loadCatalogPickerDataFromJson(): Promise<CatalogPickerData> {
  const url = "/src-tauri/resources/HX_ModelCatalog.json";
  const res = await fetch(url);
  if (!res.ok) {
    console.warn("HX_ModelCatalog.json : chargement picker impossible.", res.status);
    return {
      categories: [],
      subcategoriesByCategory: new Map(),
      modelsByCategoryAndSub: new Map(),
    };
  }
  const raw = await res.text();
  const data = JSON.parse(raw) as { models?: unknown[]; categories?: unknown[] };

  const rows: { id: string; name: string; category: string; subKey: string }[] = [];

  const pushModel = (catName: string, m: unknown) => {
    if (!m || typeof m !== "object") return;
    const mo = m as {
      id?: string | number;
      name?: string;
      presetMeta?: PresetMetaJson;
    };
    const idRaw = mo.id;
    const id =
      typeof idRaw === "string"
        ? idRaw.trim()
        : typeof idRaw === "number"
          ? String(idRaw)
          : "";
    if (!id || id === "None") return;
    const name = typeof mo.name === "string" ? mo.name.trim() : "";
    if (!name) return;
    const cat = catName.trim() || "Unknown";
    if (cat.toLowerCase() === "none") return;
    for (const subKey of presetMetaSubCategoryPickerKeys(mo.presetMeta)) {
      rows.push({ id, name, category: cat, subKey });
    }
  };

  if (Array.isArray(data.models) && data.models.length > 0) {
    for (const m of data.models) {
      const mo = m as { presetMeta?: PresetMetaJson };
      const cn = mo.presetMeta?.categoryName;
      const catName =
        typeof cn === "string" && cn.trim().length > 0 ? cn.trim() : "Unknown";
      pushModel(catName, m);
    }
  } else {
    for (const cat of data.categories ?? []) {
      if (!cat || typeof cat !== "object") continue;
      const c = cat as { name?: string; models?: unknown; subcategories?: unknown[] };
      const cn = typeof c.name === "string" ? c.name.trim() : "";
      if (!cn) continue;
      if (Array.isArray(c.models)) {
        for (const m of c.models) pushModel(cn, m);
      }
      for (const sub of c.subcategories ?? []) {
        if (!sub || typeof sub !== "object") continue;
        const models = (sub as { models?: unknown }).models;
        if (Array.isArray(models)) {
          for (const m of models) pushModel(cn, m);
        }
      }
    }
  }

  const byCat = new Map<string, Map<string, CatalogPickerModelRow[]>>();
  for (const r of rows) {
    if (!byCat.has(r.category)) byCat.set(r.category, new Map());
    const mSub = byCat.get(r.category)!;
    if (!mSub.has(r.subKey)) mSub.set(r.subKey, []);
    mSub.get(r.subKey)!.push({ id: r.id, name: r.name });
  }

  const modelsByCategoryAndSub = new Map<string, CatalogPickerModelRow[]>();
  const subcategoriesByCategory = new Map<string, string[]>();
  const categorySet = new Set<string>();

  for (const [cat, subMap] of byCat) {
    categorySet.add(cat);
    const subs = [...subMap.keys()].sort((a, b) =>
      a.localeCompare(b, undefined, { sensitivity: "base" }),
    );
    subcategoriesByCategory.set(cat, subs);
    for (const sub of subs) {
      const list = subMap.get(sub) ?? [];
      const dedup = new Map<string, string>();
      for (const { id, name } of list) {
        if (!dedup.has(id)) dedup.set(id, name);
      }
      const sorted = [...dedup.entries()]
        .map(([id, name]) => ({ id, name }))
        .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
      modelsByCategoryAndSub.set(catalogPickerRowKey(cat, sub), sorted);
    }
  }

  const categories = [...categorySet].sort((a, b) =>
    a.localeCompare(b, undefined, { sensitivity: "base" }),
  );

  return { categories, subcategoriesByCategory, modelsByCategoryAndSub };
}

export async function getCatalogPickerData(): Promise<CatalogPickerData> {
  if (!catalogPickerDataPromise) {
    catalogPickerDataPromise = loadCatalogPickerDataFromJson().catch((e) => {
      catalogPickerDataPromise = null;
      throw e;
    });
  }
  return catalogPickerDataPromise;
}
