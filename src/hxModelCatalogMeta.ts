/**
 * Jointure hex / métadonnées affichage : `HX_ModelUsbAssign.json` (`chainHexHint`, category, …).
 * Ordre des params UI : fichiers `.models` (pas de fetch runtime du catalogue).
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

export async function getPresetMetaForModel(
  slotCategory: string,
  modelDisplayName: string,
): Promise<PresetMetaJson | null> {
  const idx = await getUsbAssignIndexes();
  const entry = idx.byCategoryAndName.get(catalogKey(slotCategory, modelDisplayName));
  if (!entry) return null;
  return buildPresetMetaFromAssignEntries(idx.byId.get(entry.id) ?? [entry]);
}

export async function getCatalogModelIdForModel(
  slotCategory: string,
  modelDisplayName: string,
): Promise<string | null> {
  const idx = await getUsbAssignIndexes();
  return idx.byCategoryAndName.get(catalogKey(slotCategory, modelDisplayName))?.id ?? null;
}

type UsbAssignModelEntry = {
  id: string;
  variant: string;
  name: string;
  category: string;
  subCategory: string;
  chainHexHint: string;
  bulkHex?: string;
  basedOn?: string;
  image?: string;
};

type UsbAssignIndexes = {
  byHexHint: Map<string, UsbAssignModelEntry[]>;
  byId: Map<string, UsbAssignModelEntry[]>;
  byIdVariant: Map<string, UsbAssignModelEntry>;
  byCategoryAndName: Map<string, UsbAssignModelEntry>;
};

let usbAssignIndexesPromise: Promise<UsbAssignIndexes> | null = null;

type UsbAssignModelsFileMaps = {
  byCategory: Map<string, string[]>;
  byId: Map<string, string[]>;
};

let usbAssignModelsFileMapsPromise: Promise<UsbAssignModelsFileMaps> | null = null;

function parseModelsFileRef(raw: unknown): string[] {
  if (typeof raw === "string") {
    const t = raw.trim();
    return t ? [t] : [];
  }
  if (Array.isArray(raw)) {
    return raw
      .map((x) => (typeof x === "string" ? x.trim() : ""))
      .filter(Boolean);
  }
  return [];
}

async function loadUsbAssignModelsFileMaps(): Promise<UsbAssignModelsFileMaps> {
  const url = "/src-tauri/resources/HX_ModelUsbAssign.json";
  const res = await fetch(url);
  const byCategory = new Map<string, string[]>();
  const byId = new Map<string, string[]>();
  if (!res.ok) {
    console.warn("HX_ModelUsbAssign.json : modelsFileByCategory inaccessible.", res.status);
    return { byCategory, byId };
  }
  const data = JSON.parse(await res.text()) as {
    modelsFileByCategory?: Record<string, unknown>;
    modelsFileById?: Record<string, unknown>;
  };
  for (const [cat, ref] of Object.entries(data.modelsFileByCategory ?? {})) {
    const bases = parseModelsFileRef(ref);
    if (bases.length > 0) byCategory.set(cat.trim(), bases);
  }
  for (const [id, ref] of Object.entries(data.modelsFileById ?? {})) {
    const bases = parseModelsFileRef(ref);
    if (bases.length > 0) byId.set(id.trim(), bases);
  }
  return { byCategory, byId };
}

async function getUsbAssignModelsFileMaps(): Promise<UsbAssignModelsFileMaps> {
  if (!usbAssignModelsFileMapsPromise) {
    usbAssignModelsFileMapsPromise = loadUsbAssignModelsFileMaps().catch((e) => {
      usbAssignModelsFileMapsPromise = null;
      throw e;
    });
  }
  return usbAssignModelsFileMapsPromise;
}

/**
 * Bases `.models` (sans extension) depuis `HX_ModelUsbAssign.json` :
 * `modelsFileById` puis `modelsFileByCategory`. `null` si non déclaré.
 */
export async function modelsDefinitionFileBasesFromUsbAssign(
  catalogModelId: string,
  categoryName: string,
): Promise<string[] | null> {
  const maps = await getUsbAssignModelsFileMaps();
  const id = catalogModelId.trim();
  if (id) {
    const byId = maps.byId.get(id);
    if (byId?.length) return byId;
  }
  const cat = categoryName.trim();
  if (cat) {
    const byCat = maps.byCategory.get(cat);
    if (byCat?.length) return byCat;
  }
  return null;
}

/** `amp+cab` / `amp+cab-legacy` reprennent le fil ampli — pas d’index hex autonome. */
function chainHexHintIndexEligible(variant: string): boolean {
  const v = variant.trim().toLowerCase();
  return v !== "amp+cab" && v !== "amp+cab-legacy";
}

function usbAssignVariantPriority(variant: string): number {
  const v = variant.trim().toLowerCase();
  if (v === "amp") return 100;
  if (v === "preamp") return 95;
  if (v === "stereo") return 50;
  if (v === "dual") return 49;
  if (v === "mono") return 48;
  if (v === "single") return 47;
  if (v === "legacy") return 46;
  return 1;
}

function normalizeSlotCategoryHint(categoryHint?: string | null): string {
  return (categoryHint ?? "").trim().toLowerCase().replace(/\s+/g, "");
}

/** Slot matériel / UI de type Amp+Cab (IR ou Legacy). */
export function isAmpCabFamilySlotCategory(categoryHint?: string | null): boolean {
  const raw = (categoryHint ?? "").trim().toLowerCase();
  const slotCat = normalizeSlotCategoryHint(categoryHint);
  return (
    slotCat === "amp+cab" ||
    slotCat === "ampcab" ||
    slotCat === "amp+cablegacy" ||
    (raw.includes("amp") && raw.includes("cab"))
  );
}

export function isAmpCabLegacySlotCategory(categoryHint?: string | null): boolean {
  const raw = (categoryHint ?? "").trim().toLowerCase();
  const slotCat = normalizeSlotCategoryHint(categoryHint);
  return slotCat === "amp+cablegacy" || (raw.includes("amp") && raw.includes("legacy"));
}

/**
 * Paire amp/cab encodée dans un bulk assign `amp+cab` / `amp+cab-legacy`
 * (`…c319` + `<amp>0x1a<cab>[0x09|padding]`).
 */
export function parseAmpCabPairFromAssignBulkHex(
  bulkHex: string | undefined,
): { ampHex: string; cabHex: string } | null {
  const hex = (bulkHex ?? "").trim().toLowerCase().replace(/\s+/g, "");
  if (!hex || hex.length % 2 !== 0) return null;
  const bytes = new Uint8Array(hex.length / 2);
  for (let i = 0; i < bytes.length; i += 1) {
    const b = parseInt(hex.slice(i * 2, i * 2 + 2), 16);
    if (Number.isNaN(b)) return null;
    bytes[i] = b;
  }
  let cursor = -1;
  for (let i = 0; i + 1 < bytes.length; i += 1) {
    if (bytes[i] === 0xc3 && bytes[i + 1] === 0x19) {
      cursor = i + 2;
      break;
    }
  }
  if (cursor < 0) return null;
  const toHex = (start: number, end: number): string =>
    Array.from(bytes.subarray(start, end))
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  while (cursor + 2 < bytes.length) {
    const relSep = bytes.subarray(cursor).findIndex((b) => b === 0x1a);
    if (relSep < 0) break;
    const ampEnd = cursor + relSep;
    const cabStart = ampEnd + 1;
    if (cabStart >= bytes.length) break;
    const rel09 = bytes.subarray(cabStart).findIndex((b) => b === 0x09);
    let cabEnd: number;
    if (rel09 >= 0) {
      cabEnd = cabStart + rel09;
    } else if (bytes[cabStart] === 0xcd && cabStart + 3 <= bytes.length) {
      // Cab IR (`cd03xx`) : souvent 3 octets puis padding `00` sans `0x09` (bulk assign).
      cabEnd = cabStart + 3;
    } else {
      const rel00 = bytes.subarray(cabStart).findIndex((b) => b === 0x00);
      cabEnd = rel00 > 0 ? cabStart + rel00 : cabStart + 1;
    }
    const ampHex = toHex(cursor, ampEnd);
    const cabHex = toHex(cabStart, cabEnd);
    if (ampHex && cabHex && ampHex !== cabHex) {
      return { ampHex, cabHex };
    }
    cursor = cabEnd + 1;
  }
  return null;
}

/** Paire amp/cab par défaut pour une entrée assign (picker / sonde USB). */
export async function ampCabHexPairFromAssignVariant(
  modelId: string | null | undefined,
  assignVariant: string,
): Promise<{ ampHex: string; cabHex: string } | null> {
  const id = (modelId ?? "").trim();
  const v = assignVariant.trim().toLowerCase();
  if (!id || (v !== "amp+cab" && v !== "amp+cab-legacy")) return null;
  const idx = await getUsbAssignIndexes();
  const hit = idx.byIdVariant.get(`${id}\0${v}`);
  return parseAmpCabPairFromAssignBulkHex(hit?.bulkHex);
}

/** Paire cab1/cab2 dans un bulk assign `dual` (`…c319` + `<cab1>0x1a<cab2>`). */
export async function cabDualHexPairFromAssignVariant(
  modelId: string | null | undefined,
  assignVariant: string,
): Promise<{ cab1Hex: string; cab2Hex: string } | null> {
  const id = (modelId ?? "").trim();
  const v = assignVariant.trim().toLowerCase();
  if (!id || v !== "dual") return null;
  const idx = await getUsbAssignIndexes();
  const hit = idx.byIdVariant.get(`${id}\0${v}`);
  const pair = parseAmpCabPairFromAssignBulkHex(hit?.bulkHex);
  if (!pair) return null;
  return { cab1Hex: pair.ampHex, cab2Hex: pair.cabHex };
}

/** Paires `cab1 + 1a + cab2` sur le fil module (preset dump ou scroll). */
function looksLikeCabDualWirePart(part: string): boolean {
  if (!part || !/^[0-9a-f]+$/.test(part)) return false;
  // Dual modern / IR : hints `cd…` (≥ 6 hex, ex. cd031b).
  if (part.startsWith("cd")) return part.length >= 6;
  // Dual legacy hybrid : hint exactement 1 octet (2 hex, ex. 33 / 30).
  return part.length === 2;
}

/** Paires `cab1 + 1a + cab2` sur le fil module (preset dump ou scroll).
 *  Ancré sur le marqueur dual `c3 19` (modern ET legacy), puis séparateur `1a`. */
export function cabDualWireParts(
  moduleHex: string | undefined,
): { cab1Hex: string; cab2Hex: string } | null {
  const hex = (moduleHex ?? "").trim().toLowerCase();
  if (!hex) return null;

  // Zone cab dual = après le marqueur `c3 19`. Repli : ancien comportement (fil = hint 1a hint).
  const marker = "c319";
  const m = hex.indexOf(marker);
  const zone = m >= 0 ? hex.slice(m + marker.length) : hex;

  const sep = "1a";
  const i = zone.indexOf(sep);
  if (i <= 0) return null;
  const cab1 = zone.slice(0, i).trim();
  let cab2 = zone.slice(i + sep.length).trim();

  // cab2 court jusqu'au prochain délimiteur de bloc (`09…`) si présent (legacy : `… 1a 30 09 …`).
  const end = cab2.indexOf("09");
  if (end > 0) cab2 = cab2.slice(0, end).trim();

  if (
    !cab1 ||
    !cab2 ||
    !looksLikeCabDualWirePart(cab1) ||
    !looksLikeCabDualWirePart(cab2)
  ) {
    return null;
  }
  return { cab1Hex: cab1, cab2Hex: cab2 };
}

export function isCabDualWireHex(moduleHex: string | undefined): boolean {
  return cabDualWireParts(moduleHex) !== null;
}

/** Partie cab d’un fil combiné `amp` + `1a` + `cab` (scroll ou preset). */
export function cabHexFromAmpCabWire(moduleHex: string | undefined): string | null {
  const hex = (moduleHex ?? "").trim().toLowerCase();
  const sep = "1a";
  const i = hex.indexOf(sep);
  if (i <= 0) return null;
  const cab = hex.slice(i + sep.length).trim();
  return cab.length > 0 ? cab : null;
}

/**
 * Variante assign scroll / picker pour la famille ampli (même `id`, fil ampli seul).
 * **Legacy** : uniquement si la partie **cab** est connue (`moduleHex` combiné ou `cabHexHint`
 * depuis le preset — ex. hybrid `34` vs IR `cd0321`). Fil ampli seul → `amp+cab` par défaut.
 */
export async function usbAssignVariantForAmpFamilyScroll(
  modelId: string,
  _moduleHex: string | undefined,
  categoryHint?: string | null,
  cabHexHint?: string | null,
): Promise<UsbAssignVariant> {
  const id = modelId.trim();
  if (!id) return "mono";
  const slotCat = normalizeSlotCategoryHint(categoryHint);
  if (slotCat === "preamp") return "preamp";
  if (isAmpCabLegacySlotCategory(categoryHint)) return "amp+cab-legacy";
  if (isAmpCabFamilySlotCategory(categoryHint)) {
    const cab = (cabHexHint ?? "").trim();
    if (cab && (await isLegacyCabChainHex(cab))) return "amp+cab-legacy";
    return "amp+cab";
  }
  return "amp";
}

function presetMetaCategoryNameFromAssign(entry: UsbAssignModelEntry): string {
  const cat = entry.category.trim();
  const variant = entry.variant.trim().toLowerCase();
  if (
    (cat === "Amp+Cab" || cat === "Amp+Cab Legacy") &&
    (variant === "amp+cab" || variant === "amp+cab-legacy")
  ) {
    return "Amp";
  }
  return cat;
}

function buildPresetMetaFromAssignEntries(entries: UsbAssignModelEntry[]): PresetMetaJson | null {
  if (entries.length === 0) return null;
  const sorted = [...entries].sort(
    (a, b) => usbAssignVariantPriority(b.variant) - usbAssignVariantPriority(a.variant),
  );
  const primary = sorted[0]!;
  const hints: string[] = [];
  const subs: string[] = [];
  const variantOrder = [
    "mono",
    "single",
    "amp",
    "preamp",
    "stereo",
    "dual",
    "legacy",
    "amp+cab",
    "amp+cab-legacy",
  ];
  const byVariant = [...entries].sort(
    (a, b) => variantOrder.indexOf(a.variant) - variantOrder.indexOf(b.variant),
  );
  for (const e of byVariant) {
    const h = e.chainHexHint.trim().toLowerCase();
    if (!h || hints.includes(h)) continue;
    hints.push(h);
    subs.push(e.subCategory.trim());
  }
  const basedOn = (primary.basedOn ?? "").trim();
  return {
    categoryName: presetMetaCategoryNameFromAssign(primary),
    chainHex: hints.length <= 1 ? (hints[0] ?? primary.chainHexHint) : hints,
    subCategory: subs.length <= 1 ? (subs[0] ?? primary.subCategory) : subs,
    basedOn: basedOn.length > 0 ? basedOn : undefined,
  };
}

async function loadUsbAssignIndexes(): Promise<UsbAssignIndexes> {
  const url = "/src-tauri/resources/HX_ModelUsbAssign.json";
  const res = await fetch(url);
  const byHexHint = new Map<string, UsbAssignModelEntry[]>();
  const byId = new Map<string, UsbAssignModelEntry[]>();
  const byIdVariant = new Map<string, UsbAssignModelEntry>();
  const byCategoryAndName = new Map<string, UsbAssignModelEntry>();
  if (!res.ok) {
    console.warn("HX_ModelUsbAssign.json : chargement impossible.", res.status);
    return { byHexHint, byId, byIdVariant, byCategoryAndName };
  }
  const data = JSON.parse(await res.text()) as { entries?: Record<string, unknown>[] };
  for (const raw of data.entries ?? []) {
    const id = typeof raw.id === "string" ? raw.id.trim() : "";
    if (!id) continue;
    const entry: UsbAssignModelEntry = {
      id,
      variant: typeof raw.variant === "string" ? raw.variant.trim().toLowerCase() : "mono",
      name: typeof raw.name === "string" ? raw.name.trim() : id,
      category: typeof raw.category === "string" ? raw.category.trim() : "Unknown",
      subCategory: typeof raw.subCategory === "string" ? raw.subCategory.trim() : "",
      chainHexHint:
        typeof raw.chainHexHint === "string" ? raw.chainHexHint.trim().toLowerCase() : "",
      bulkHex: typeof raw.bulkHex === "string" ? raw.bulkHex.trim().toLowerCase() : undefined,
      basedOn: typeof raw.basedOn === "string" ? raw.basedOn.trim() : undefined,
      image: typeof raw.image === "string" ? raw.image.trim() : undefined,
    };
    if (entry.chainHexHint && chainHexHintIndexEligible(entry.variant)) {
      const hintList = byHexHint.get(entry.chainHexHint) ?? [];
      hintList.push(entry);
      byHexHint.set(entry.chainHexHint, hintList);
    }
    const idList = byId.get(id) ?? [];
    idList.push(entry);
    byId.set(id, idList);
    byIdVariant.set(`${id}\0${entry.variant}`, entry);
    const cnKey = catalogKey(entry.category, entry.name);
    if (!byCategoryAndName.has(cnKey)) {
      byCategoryAndName.set(cnKey, entry);
    }
  }
  return { byHexHint, byId, byIdVariant, byCategoryAndName };
}

async function getUsbAssignIndexes(): Promise<UsbAssignIndexes> {
  if (!usbAssignIndexesPromise) {
    usbAssignIndexesPromise = loadUsbAssignIndexes().catch((e) => {
      usbAssignIndexesPromise = null;
      throw e;
    });
  }
  return usbAssignIndexesPromise;
}

/**
 * Jointure `chainHexHint` (`HX_ModelUsbAssign.json`) — scroll HW, preset, grille.
 * `categoryHint` (ex. « Amp+Cab ») départage amp / amp+cab sur le même fil ampli.
 */
export async function getCatalogModelIdForHex(
  moduleHex: string | undefined,
  categoryHint?: string | null,
): Promise<string | null> {
  return getCatalogModelIdFromUsbAssignHex(moduleHex, categoryHint);
}

/** Alias explicite : même résolution que `getCatalogModelIdForHex`. */
export async function getCatalogModelIdFromUsbAssignHex(
  moduleHex: string | undefined,
  categoryHint?: string | null,
): Promise<string | null> {
  const hexNorm = (moduleHex ?? "").trim().toLowerCase();
  if (!hexNorm) return null;
  const { byHexHint: byHint } = await getUsbAssignIndexes();
  const slotCat = normalizeSlotCategoryHint(categoryHint);
  const wantPreamp = slotCat === "preamp";
  const hints: string[] = [hexNorm];
  const sep = "1a";
  const i = hexNorm.indexOf(sep);
  if (i > 0) {
    const ampPart = hexNorm.slice(0, i);
    if (ampPart.length > 0) hints.push(ampPart);
  }
  for (const hint of hints) {
    const entries = byHint.get(hint);
    if (!entries?.length) continue;
    const hit =
      (wantPreamp ? entries.find((e) => e.variant === "preamp") : undefined) ??
      entries.find((e) => e.variant === "amp") ??
      entries[0];
    const id = hit?.id.trim();
    if (id) return id;
  }
  return null;
}

/** Nom de fichier `image` du catalogue (ex. `FX_HX_EQ_SimpleTilt.png`), ou `null`. */
export async function getCatalogModelImageForModel(
  slotCategory: string,
  modelDisplayName: string,
): Promise<string | null> {
  const idx = await getUsbAssignIndexes();
  const img = idx.byCategoryAndName.get(catalogKey(slotCategory, modelDisplayName))?.image;
  const t = (img ?? "").trim();
  return t.length > 0 ? t : null;
}

export async function getPresetMetaForId(modelId: string | null | undefined): Promise<PresetMetaJson | null> {
  const id = (modelId ?? "").trim();
  if (!id) return null;
  const idx = await getUsbAssignIndexes();
  return buildPresetMetaFromAssignEntries(idx.byId.get(id) ?? []);
}

export async function getCatalogModelImageForId(modelId: string | null | undefined): Promise<string | null> {
  const id = (modelId ?? "").trim();
  if (!id) return null;
  const idx = await getUsbAssignIndexes();
  const entries = idx.byId.get(id) ?? [];
  const img = entries.find((e) => (e.image ?? "").trim())?.image;
  const t = (img ?? "").trim();
  return t.length > 0 ? t : null;
}

export async function getCatalogModelNameForId(modelId: string | null | undefined): Promise<string | null> {
  const id = (modelId ?? "").trim();
  if (!id) return null;
  const idx = await getUsbAssignIndexes();
  const entries = idx.byId.get(id) ?? [];
  const name = entries[0]?.name ?? "";
  const t = name.trim();
  return t.length > 0 ? t : null;
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
/**
 * Variante pour `HX_ModelUsbAssign.json` / sonde USB : `legacy` si le preset l'indique,
 * sinon `pickSignal` (mono/stereo), repli `mono`.
 */
/** `chainHexHint` assign pour une variante USB (mono/stereo/amp+cab/…). */
export async function moduleHexForUsbVariant(
  modelId: string | null | undefined,
  assignVariant: string,
  meta?: PresetMetaJson | null,
): Promise<string | null> {
  const id = (modelId ?? "").trim();
  const v = assignVariant.trim().toLowerCase();
  if (id) {
    const idx = await getUsbAssignIndexes();
    const hit = idx.byIdVariant.get(`${id}\0${v}`);
    if (hit?.chainHexHint) return hit.chainHexHint;
  }
  return moduleHexFromCatalogForUsbVariant(meta ?? null, assignVariant);
}

/** Repli via `presetMeta.chainHex` (tableau mono/stéréo). */
export function moduleHexFromCatalogForUsbVariant(
  meta: PresetMetaJson | null,
  assignVariant: string,
): string | null {
  if (!meta) return null;
  const v = assignVariant.trim().toLowerCase();
  const hexList = normalizeHexList(meta.chainHex);
  if (hexList.length === 0) return null;
  if (v === "stereo" && hexList.length >= 2) return hexList[1] ?? null;
  return hexList[0] ?? null;
}

export type UsbAssignVariant =
  | "mono"
  | "stereo"
  | "legacy"
  | "single"
  | "dual"
  | "amp"
  | "preamp"
  | "amp+cab"
  | "amp+cab-legacy"
  | "sendReturn";

/** Cab hybrid pré-3.50 (`cab.models`, subCategory Legacy) vs cab IR Single/Dual. */
export async function isLegacyCabChainHex(cabHex: string): Promise<boolean> {
  const hex = (cabHex ?? "").trim().toLowerCase();
  if (!hex) return false;
  const { byHexHint } = await getUsbAssignIndexes();
  for (const hint of moduleHexCatalogLookupCandidates(hex)) {
    const entries = byHexHint.get(hint);
    if (!entries?.length) continue;
    for (const e of entries) {
      const cat = e.category.trim().toLowerCase();
      const sub = e.subCategory.trim().toLowerCase();
      const variant = e.variant.trim().toLowerCase();
      if (cat === "cab" && (sub === "legacy" || variant === "legacy")) return true;
    }
  }
  return false;
}

/** Variante assign pour slot Amp+Cab : cab IR (`amp+cab`) vs hybrid legacy (`amp+cab-legacy`). */
export async function usbAssignVariantForAmpCabSlot(
  meta: PresetMetaJson | null,
  moduleHex: string | undefined,
  slotCategory?: string | null,
  modelId?: string | null,
  cabHexHint?: string | null,
): Promise<UsbAssignVariant> {
  const id = (modelId ?? "").trim();
  const slotCat = normalizeSlotCategoryHint(slotCategory);
  const catalogCat = (meta?.categoryName ?? "").trim().toLowerCase();
  if (
    id &&
    (isAmpCabFamilySlotCategory(slotCategory) ||
      slotCat === "amp" ||
      slotCat === "preamp") &&
    (catalogCat === "amp" || catalogCat === "preamp")
  ) {
    return usbAssignVariantForAmpFamilyScroll(id, moduleHex, slotCategory, cabHexHint);
  }
  const base = usbAssignVariantFromPresetMeta(meta, moduleHex, slotCategory);
  if (base !== "amp+cab" || !id) return base;
  return usbAssignVariantForAmpFamilyScroll(id, moduleHex, slotCategory, cabHexHint);
}

export function usbAssignVariantFromPresetMeta(
  meta: PresetMetaJson | null,
  moduleHex: string | undefined,
  /** Catégorie du slot lu sur le preset / matériel (ex. « Amp+Cab »). */
  slotCategory?: string | null,
): UsbAssignVariant {
  const slotCat = normalizeSlotCategoryHint(slotCategory);
  if (slotCat === "send/return") return "sendReturn";
  const catalogCat = (meta?.categoryName ?? "").trim().toLowerCase();
  if (isAmpCabLegacySlotCategory(slotCategory)) return "amp+cab-legacy";
  if (isAmpCabFamilySlotCategory(slotCategory) && (catalogCat === "amp" || catalogCat === "preamp")) {
    return "amp+cab";
  }
  if (catalogCat === "amp") {
    return "amp";
  }
  if (catalogCat === "preamp") {
    return "preamp";
  }
  if (catalogCat === "send/return") {
    return "sendReturn";
  }
  const sc = meta?.subCategory;
  const bits: string[] = [];
  if (typeof sc === "string") bits.push(sc);
  else if (Array.isArray(sc)) {
    for (const x of sc) {
      if (typeof x === "string") bits.push(x);
    }
  }
  const joined = bits.join(" ").toLowerCase();
  if (catalogCat === "cab") {
    if (joined.includes("dual")) return "dual";
    if (joined.includes("single")) return "single";
  }
  if (joined.includes("legacy")) {
    if (catalogCat === "cab") {
      const sig = pickSignal(meta, moduleHex);
      return sig === "stereo" ? "dual" : "single";
    }
    return "legacy";
  }
  const sig = pickSignal(meta, moduleHex);
  if (sig === "stereo") return "stereo";
  if (sig === "mono") return "mono";
  return "mono";
}

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
  usbAssignPickerDataPromise = null;
  usbAssignIndexesPromise = null;
  usbAssignModelsFileMapsPromise = null;
}

// --- Sélecteur visuel catégorie / sous-catégorie / modèle (aperçu, pas d’écriture USB) ---

export type CatalogPickerModelRow = {
  id: string;
  name: string;
  /** Ligne `HX_ModelUsbAssign.json` : `mono` | `stereo` | `legacy` (pour l’invoke sonde). */
  assignVariant?: string;
  /** Source I/O Path 1 (`ioSources[]`) — écriture `write_path1_input_source`, pas `probe_slot_model_usb`. */
  ioSource?: boolean;
  /** PNG `icons_models/` (ioSources[]). */
  image?: string;
  /** Type Split Path 1 (`splitSources[]`) — écriture `write_path1_split_type`. */
  splitSource?: boolean;
  catalogModelId?: string;
  parentModelId?: string;
  wireValue?: number;
  /** Noms device Rust (`SUPPORTED_DEVICES`) ou sous-chaîne tolérée (ex. `stomp`). */
  devices?: string[];
};

export type CatalogPickerData = {
  /** Catégories (ordre catalogue trié, ou ordre fichier pour le picker USB). */
  categories: string[];
  /** Sous-clés par catégorie (triées pour le catalogue ; ordre d’apparition pour le picker USB). */
  subcategoriesByCategory: Map<string, string[]>;
  /** Clé = `catalogPickerRowKey(cat, sub)` → liste de modèles (tri par nom catalogue, ou ordre fichier USB). */
  modelsByCategoryAndSub: Map<string, CatalogPickerModelRow[]>;
};

export function catalogPickerRowKey(category: string, subKey: string): string {
  return `${category.trim()}\0${subKey}`;
}

/**
 * Libellé sous-catégorie picker (peut différer de `subCategory` assign).
 * Cab Legacy : une entrée assign `Legacy` + variant `single`|`dual` → « Single Legacy » / « Dual Legacy ».
 */
export function usbAssignPickerSubDisplay(
  category: string,
  subCategory: string,
  variant: string,
): string {
  const cat = category.trim();
  const sub = subCategory.trim();
  const v = variant.trim().toLowerCase();
  if (cat === "Cab" && sub.toLowerCase() === "legacy") {
    return v === "dual" ? "Dual Legacy" : "Single Legacy";
  }
  return sub || "Mono";
}

// --- Picker slot : uniquement les modèles présents dans HX_ModelUsbAssign.json (ordre hardware) ---

let usbAssignPickerDataPromise: Promise<CatalogPickerData> | null = null;

type UsbAssignFileEntry = {
  id?: string;
  variant?: string;
  name?: string;
  category?: string;
  subCategory?: string;
};

type UsbAssignIoSourceEntry = {
  id?: string;
  parentModelId?: string;
  name?: string;
  category?: string;
  subCategory?: string;
  wireValue?: number;
  devices?: string[];
  image?: string;
};

type UsbAssignSplitSourceEntry = {
  id?: string;
  catalogModelId?: string;
  name?: string;
  category?: string;
  subCategory?: string;
  wireValue?: number;
  devices?: string[];
};

async function loadUsbAssignPickerDataFromJson(): Promise<CatalogPickerData> {
  const url = "/src-tauri/resources/HX_ModelUsbAssign.json";
  const res = await fetch(url);
  if (!res.ok) {
    console.warn("HX_ModelUsbAssign.json : chargement picker impossible.", res.status);
    return {
      categories: [],
      subcategoriesByCategory: new Map(),
      modelsByCategoryAndSub: new Map(),
    };
  }
  const data = JSON.parse(await res.text()) as {
    entries?: UsbAssignFileEntry[];
    ioSources?: UsbAssignIoSourceEntry[];
    splitSources?: UsbAssignSplitSourceEntry[];
    pickerExcludedCategories?: string[];
  };
  const entries = Array.isArray(data.entries) ? data.entries : [];
  const ioSources = Array.isArray(data.ioSources) ? data.ioSources : [];
  const splitSources = Array.isArray(data.splitSources) ? data.splitSources : [];
  const pickerExcluded = new Set(
    (Array.isArray(data.pickerExcludedCategories) ? data.pickerExcludedCategories : [])
      .map((c) => (typeof c === "string" ? c.trim() : ""))
      .filter(Boolean),
  );

  const categories: string[] = [];
  const seenCat = new Set<string>();
  const subcategoriesByCategory = new Map<string, string[]>();
  const seenSubByCat = new Map<string, Set<string>>();
  const modelsByCategoryAndSub = new Map<string, CatalogPickerModelRow[]>();

  const pushSub = (cat: string, sub: string) => {
    if (!seenSubByCat.has(cat)) seenSubByCat.set(cat, new Set());
    const st = seenSubByCat.get(cat)!;
    if (st.has(sub)) return;
    st.add(sub);
    const arr = subcategoriesByCategory.get(cat) ?? [];
    arr.push(sub);
    subcategoriesByCategory.set(cat, arr);
  };

  for (const e of entries) {
    const id = (e.id ?? "").trim();
    if (!id) continue;
    const cat = (e.category ?? "Unknown").trim() || "Unknown";
    if (pickerExcluded.has(cat)) continue;
    const subRaw = (e.subCategory ?? "Mono").trim() || "Mono";
    const assignVariant = (e.variant ?? "mono").trim().toLowerCase();
    const sub = usbAssignPickerSubDisplay(cat, subRaw, assignVariant);
    const name = (e.name ?? id).trim() || id;
    if (!seenCat.has(cat)) {
      seenCat.add(cat);
      categories.push(cat);
    }
    pushSub(cat, sub);
    const key = catalogPickerRowKey(cat, sub);
    const list = modelsByCategoryAndSub.get(key) ?? [];
    list.push({ id, name, assignVariant });
    modelsByCategoryAndSub.set(key, list);
  }

  for (const src of ioSources) {
    const id = (src.id ?? "").trim();
    const parentModelId = (src.parentModelId ?? "").trim();
    if (!id || !parentModelId) continue;
    const cat = (src.category ?? "Input").trim() || "Input";
    if (pickerExcluded.has(cat)) continue;
    const subRaw = (src.subCategory ?? "Source").trim() || "Source";
    const sub = usbAssignPickerSubDisplay(cat, subRaw, "mono");
    const name = (src.name ?? id).trim() || id;
    const devices = Array.isArray(src.devices)
      ? src.devices.map((d) => String(d).trim()).filter(Boolean)
      : undefined;
    if (!seenCat.has(cat)) {
      seenCat.add(cat);
      categories.push(cat);
    }
    pushSub(cat, sub);
    const key = catalogPickerRowKey(cat, sub);
    const list = modelsByCategoryAndSub.get(key) ?? [];
    list.push({
      id,
      name,
      ioSource: true,
      parentModelId,
      wireValue: typeof src.wireValue === "number" ? src.wireValue : undefined,
      devices,
      image: typeof src.image === "string" ? src.image.trim() || undefined : undefined,
    });
    modelsByCategoryAndSub.set(key, list);
  }

  for (const src of splitSources) {
    const id = (src.id ?? "").trim();
    const catalogModelId = (src.catalogModelId ?? "").trim();
    if (!id || !catalogModelId) continue;
    const cat = (src.category ?? "Split").trim() || "Split";
    // splitSources[] : toujours enregistrés (comme ioSources[] pour Input).
    // pickerExcluded ne s'applique qu'aux entrées FX `entries[]`.
    const subRaw = (src.subCategory ?? "Mono").trim() || "Mono";
    const sub = usbAssignPickerSubDisplay(cat, subRaw, "mono");
    const name = (src.name ?? id).trim() || id;
    const devices = Array.isArray(src.devices)
      ? src.devices.map((d) => String(d).trim()).filter(Boolean)
      : undefined;
    if (!seenCat.has(cat)) {
      seenCat.add(cat);
      categories.push(cat);
    }
    pushSub(cat, sub);
    const key = catalogPickerRowKey(cat, sub);
    const list = modelsByCategoryAndSub.get(key) ?? [];
    list.push({
      id,
      name,
      splitSource: true,
      catalogModelId,
      wireValue: typeof src.wireValue === "number" ? src.wireValue : undefined,
      devices,
    });
    modelsByCategoryAndSub.set(key, list);
  }

  return { categories, subcategoriesByCategory, modelsByCategoryAndSub };
}

/** Filtre device connecté pour `ioSources[]` (tolère « stomp », « HX Stomp XL », etc.). */
export function ioSourceMatchesConnectedDevice(
  devices: string[] | undefined,
  connectedDeviceName: string | null,
): boolean {
  if (!devices || devices.length === 0) return true;
  const n = (connectedDeviceName ?? "").trim().toLowerCase();
  if (!n) return true;
  return devices.some((d) => {
    const t = d.trim().toLowerCase();
    return t && (n.includes(t) || t.includes(n));
  });
}

/**
 * Valeur wire Stomp compacte (1 / 4 / 6) depuis l’enum preset ou la chaîne lue.
 * Le dump preset peut utiliser l’enum `input_type` complet (ex. 11 = Return 1/2, 15 = USB 5/6).
 */
export function normalizeStompInputWireValue(raw: number): number {
  const v = Math.round(raw);
  if (v === 11) return 4;
  if (v === 15) return 6;
  return v;
}

/** Ligne `ioSources[]` pour une valeur wire `@input` Stomp (ex. 1 / 4 / 6). */
export function findIoSourceRowByWireValue(
  data: CatalogPickerData,
  parentModelId: string,
  wireValue: number,
  connectedDeviceName: string | null,
): CatalogPickerModelRow | null {
  const parent = parentModelId.trim();
  const wire = normalizeStompInputWireValue(wireValue);
  if (!parent || !Number.isFinite(wire)) return null;
  const key = catalogPickerRowKey("Input", "Source");
  const rows = data.modelsByCategoryAndSub.get(key) ?? [];
  return (
    rows.find(
      (r) =>
        r.ioSource &&
        (r.parentModelId ?? "").trim() === parent &&
        r.wireValue === wire &&
        ioSourceMatchesConnectedDevice(r.devices, connectedDeviceName),
    ) ?? null
  );
}

/** Ligne `ioSources[]` par id (ex. `HelixStomp_Input_MainLR`). */
export function findIoSourceRowById(
  data: CatalogPickerData,
  ioSourceId: string,
): CatalogPickerModelRow | null {
  const id = ioSourceId.trim();
  if (!id) return null;
  const key = catalogPickerRowKey("Input", "Source");
  const rows = data.modelsByCategoryAndSub.get(key) ?? [];
  return rows.find((r) => r.ioSource && r.id === id) ?? null;
}

/** Id `ioSources[]` pour une valeur wire `@input` Stomp (ex. 1 / 4 / 6). */
export function findIoSourceIdByWireValue(
  data: CatalogPickerData,
  parentModelId: string,
  wireValue: number,
  connectedDeviceName: string | null,
): string | null {
  return findIoSourceRowByWireValue(data, parentModelId, wireValue, connectedDeviceName)?.id ?? null;
}

type ChainParamValueJson = boolean | number | string;

/** Résout la source Input surlignée depuis les valeurs chaîne preset / alignées. */
export function findIoSourceIdFromInputChainValues(
  data: CatalogPickerData,
  parentModelId: string,
  chainValues: readonly (ChainParamValueJson | undefined)[] | null | undefined,
  inputParamChainIndex: number,
  connectedDeviceName: string | null,
): string | null {
  const parent = parentModelId.trim();
  if (!parent || !chainValues?.length) return null;

  const key = catalogPickerRowKey("Input", "Source");
  const knownWires = new Set(
    (data.modelsByCategoryAndSub.get(key) ?? [])
      .filter(
        (r) =>
          r.ioSource &&
          (r.parentModelId ?? "").trim() === parent &&
          ioSourceMatchesConnectedDevice(r.devices, connectedDeviceName) &&
          typeof r.wireValue === "number",
      )
      .map((r) => r.wireValue as number),
  );
  if (knownWires.size === 0) return null;

  const candidates: number[] = [];
  const push = (v: ChainParamValueJson | undefined) => {
    if (typeof v !== "number" || !Number.isFinite(v)) return;
    const w = normalizeStompInputWireValue(v);
    if (!candidates.includes(w)) candidates.push(w);
  };
  push(chainValues[inputParamChainIndex]);
  for (const v of chainValues) push(v);

  for (const wire of candidates) {
    if (!knownWires.has(wire)) continue;
    const id = findIoSourceIdByWireValue(data, parent, wire, connectedDeviceName);
    if (id) return id;
  }
  return null;
}

const SPLIT_CHAIN_HEX_TO_WIRE: Record<string, number> = {
  "6ccd0023": 0,
  "6ccd0024": 1,
  "6ccd0025": 2,
  "6ccd0026": 0x33,
};

/** Valeur wire type Split depuis chainHex preset (ex. `6ccd0024` → A/B). */
export function splitWireFromChainHex(moduleHex: string | null | undefined): number | null {
  const h = (moduleHex ?? "").trim().toLowerCase().replace(/[^0-9a-f]/g, "");
  if (!h) return null;
  const tail = h.length >= 8 ? h.slice(-8) : h;
  const wire = SPLIT_CHAIN_HEX_TO_WIRE[tail];
  return wire !== undefined ? wire : null;
}

export function splitChainHexFromWire(wire: number): string {
  const m: Record<number, string> = {
    0: "6ccd0023",
    1: "6ccd0024",
    2: "6ccd0025",
    0x33: "6ccd0026",
  };
  return m[wire] ?? "";
}

/** Id `splitSources[]` pour une valeur wire Split Path 1 (0 / 1 / 2 / 0x33). */
export function findSplitSourceIdByWireValue(
  data: CatalogPickerData,
  wire: number,
  connectedDeviceName: string | null,
): string | null {
  for (const rows of data.modelsByCategoryAndSub.values()) {
    for (const r of rows) {
      if (
        r.splitSource &&
        r.wireValue === wire &&
        ioSourceMatchesConnectedDevice(r.devices, connectedDeviceName)
      ) {
        return r.id;
      }
    }
  }
  return null;
}

export function findSplitSourceIdByCatalogModelId(
  data: CatalogPickerData,
  catalogModelId: string,
  connectedDeviceName: string | null,
): string | null {
  const id = catalogModelId.trim();
  if (!id) return null;
  for (const rows of data.modelsByCategoryAndSub.values()) {
    for (const r of rows) {
      if (
        r.splitSource &&
        (r.catalogModelId ?? "").trim() === id &&
        ioSourceMatchesConnectedDevice(r.devices, connectedDeviceName)
      ) {
        return r.id;
      }
    }
  }
  return null;
}

/** Données picker slot : catégories / sous-catégories / modèles issus de `HX_ModelUsbAssign.json` (ordre fichier). */
export async function getUsbAssignPickerData(): Promise<CatalogPickerData> {
  if (!usbAssignPickerDataPromise) {
    usbAssignPickerDataPromise = loadUsbAssignPickerDataFromJson().catch((e) => {
      usbAssignPickerDataPromise = null;
      throw e;
    });
  }
  return usbAssignPickerDataPromise;
}

/** Repère la combinaison catégorie + sous-clé pour un `id` + variante USB (sync depuis le slot chargé). */
export function findUsbAssignPickerLocation(
  data: CatalogPickerData,
  catalogModelId: string,
  assignVariant: string,
  /** Priorité catégorie picker (ex. « Amp+Cab Legacy ») si connue. */
  preferCategory?: string | null,
): { category: string; subKey: string } | null {
  const id = catalogModelId.trim();
  const vid = assignVariant.trim().toLowerCase();
  if (!id) return null;
  const prefer = (preferCategory ?? "").trim();
  const cats = prefer && data.categories.includes(prefer)
    ? [prefer, ...data.categories.filter((c) => c !== prefer)]
    : data.categories;
  for (const cat of cats) {
    const subs = data.subcategoriesByCategory.get(cat) ?? [];
    for (const sub of subs) {
      const key = catalogPickerRowKey(cat, sub);
      const rows = data.modelsByCategoryAndSub.get(key) ?? [];
      const hit = rows.some(
        (r) => r.id === id && (r.assignVariant ?? "").toLowerCase() === vid,
      );
      if (hit) return { category: cat, subKey: sub };
    }
  }
  return null;
}

/** Repère un modèle Cab dans le picker (essai `single` puis `legacy` puis `dual`). */
export function findCabModelPickerLocation(
  data: CatalogPickerData,
  catalogModelId: string,
  preferCategory?: string | null,
): { category: string; subKey: string; assignVariant: string } | null {
  for (const assignVariant of ["single", "legacy", "dual"] as const) {
    const loc = findUsbAssignPickerLocation(
      data,
      catalogModelId,
      assignVariant,
      preferCategory ?? "Cab",
    );
    if (loc) return { ...loc, assignVariant };
  }
  return null;
}

/** ID catalogue Cab 2 d’un slot dual (`cd02d6` usine → 2x12 Jazz Rivet, pas le dual Cab 1). */
export async function getCatalogModelIdForCabDualCab2Hex(
  _dualCatalogModelId: string,
  cab2Hex: string | undefined,
  _cab1Hex?: string | null,
): Promise<string | null> {
  const h2 = (cab2Hex ?? "").trim().toLowerCase();
  if (!h2) return null;
  let resolved = (await getCatalogModelIdForCabSingleHex(h2))?.trim() ?? null;
  if (!resolved) return null;
  // Suffixe stéréo usine (`cd02d6`) pointe souvent vers la ligne `dual` *WithPan* :
  // le picker Cab 2 doit afficher le single IR (ex. 2x12 Jazz Rivet).
  if (resolved.endsWith("WithPan")) {
    const bare = resolved.replace(/WithPan$/, "");
    if (bare) {
      const idx = await getUsbAssignIndexes();
      if (idx.byIdVariant.has(`${bare}\0single`)) {
        return bare;
      }
    }
  }
  return resolved;
}

/** ID catalogue d’un cab **single** (ou legacy) pour un `chainHex` — évite le repli `dual` (ex. `cd02d6` pan). */
export async function getCatalogModelIdForCabSingleHex(
  moduleHex: string | undefined,
): Promise<string | null> {
  const hexNorm = (moduleHex ?? "").trim().toLowerCase();
  if (!hexNorm) return null;
  const { byHexHint } = await getUsbAssignIndexes();
  const entries = byHexHint.get(hexNorm);
  if (!entries?.length) return null;
  const cabEntries = entries.filter((e) => e.category.trim().toLowerCase() === "cab");
  const hit =
    cabEntries.find((e) => e.variant === "single") ??
    cabEntries.find((e) => e.variant === "legacy") ??
    cabEntries[0];
  const id = hit?.id.trim();
  return id || null;
}
