/**
 * Lecture locale de `HX_ModelCatalog.json` pour `presetMeta` (canal, signal, etc.).
 */

export type PresetMetaJson = {
  chainHex?: string | string[];
  channel?: string;
  signal?: string | string[];
  instrument?: string | string[];
  emulationName?: string;
};

let metaMapPromise: Promise<Map<string, PresetMetaJson>> | null = null;

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

async function loadPresetMetaMap(): Promise<Map<string, PresetMetaJson>> {
  const url = "/src-tauri/resources/HX_ModelCatalog.json";
  const res = await fetch(url);
  if (!res.ok) {
    console.warn("HX_ModelCatalog.json : chargement presetMeta impossible.", res.status);
    return new Map();
  }
  const raw = await res.text();
  const data = JSON.parse(raw) as { categories?: unknown[] };
  const map = new Map<string, PresetMetaJson>();
  const record = (catName: string, models: unknown) => {
    if (!Array.isArray(models)) return;
    for (const m of models) {
      if (!m || typeof m !== "object") continue;
      const mo = m as { name?: string; presetMeta?: PresetMetaJson };
      const name = typeof mo.name === "string" ? mo.name.trim() : "";
      if (!name || !mo.presetMeta) continue;
      const key = catalogKey(catName, name);
      if (!map.has(key)) map.set(key, { ...mo.presetMeta });
    }
  };
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
  return map;
}

export async function getPresetMetaForModel(
  slotCategory: string,
  modelDisplayName: string,
): Promise<PresetMetaJson | null> {
  if (!metaMapPromise) {
    metaMapPromise = loadPresetMetaMap().catch((e) => {
      metaMapPromise = null;
      throw e;
    });
  }
  const map = await metaMapPromise;
  return map.get(catalogKey(slotCategory, modelDisplayName)) ?? null;
}

export function pickChannel(meta: PresetMetaJson | null): string | null {
  const c = meta?.channel;
  if (typeof c !== "string") return null;
  const t = c.trim();
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
    const idx = hexList.indexOf(hexNorm);
    if (idx >= 0 && idx < sig.length) {
      const s = sig[idx];
      if (typeof s === "string" && s.trim()) return s.trim();
    }
  }
  for (const x of sig) {
    if (typeof x === "string" && x.trim()) return x.trim();
  }
  return null;
}

/** Tests uniquement. */
export function resetHxCatalogMetaMapForTests(): void {
  metaMapPromise = null;
}
