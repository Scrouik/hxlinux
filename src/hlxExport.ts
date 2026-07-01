/** Assemblage document `L6Preset` v6 (export .hlx). */

export type HlxFxBlockExport = {
  catalogModelId: string;
  path: 0 | 1;
  position: number;
  blockType: 0 | 1;
  enabled: boolean;
  stereo: boolean | null;
  bypassVolume: number | null;
  params: Record<string, boolean | number | string>;
};

export type HlxFlowBlockExport = {
  catalogModelId: string;
  position: number;
  enabled: boolean;
  params: Record<string, boolean | number | string>;
};

export type HlxSnapshotBlockStates = Record<string, boolean>;

export type HlxExportInput = {
  presetName: string;
  deviceId?: number;
  deviceVersion?: number;
  appVersion?: number;
  topology0: string;
  fxBlocks: HlxFxBlockExport[];
  inputA: HlxFlowBlockExport;
  inputB: HlxFlowBlockExport;
  split: HlxFlowBlockExport;
  join: HlxFlowBlockExport;
  outputA: HlxFlowBlockExport;
  outputB: HlxFlowBlockExport;
  snapshotBlockStates: HlxSnapshotBlockStates;
  /** États bloc par snapshot (index 0…3). Si absent, `snapshotBlockStates` est répliqué. */
  snapshotBlockStatesByIndex?: HlxSnapshotBlockStates[];
  snapshotCount?: number;
};

export const HLX_DEVICE_ID_STOMP_XL = 2162699;

export function sanitizeHlxFilename(name: string): string {
  const base = name
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
    .replace(/\s+/g, " ")
    .trim();
  return base.length > 0 ? base : "preset";
}

export function buildL6PresetDocument(input: HlxExportInput): Record<string, unknown> {
  const now = Math.floor(Date.now() / 1000);
  const dsp0: Record<string, unknown> = {};

  dsp0.inputA = flowBlockToHlx(input.inputA);
  dsp0.inputB = flowBlockToHlx(input.inputB);
  dsp0.split = flowBlockToHlx(input.split);
  dsp0.join = flowBlockToHlx(input.join);
  dsp0.outputA = flowBlockToHlx(input.outputA);
  dsp0.outputB = flowBlockToHlx(input.outputB);

  input.fxBlocks.forEach((block, i) => {
    dsp0[`block${i}`] = fxBlockToHlx(block);
  });

  const snapshotCount = input.snapshotCount ?? 4;
  const snapshotStatesList =
    input.snapshotBlockStatesByIndex && input.snapshotBlockStatesByIndex.length > 0
      ? input.snapshotBlockStatesByIndex
      : Array.from({ length: snapshotCount }, () => input.snapshotBlockStates);
  const tone: Record<string, unknown> = {
    dsp0,
    dsp1: {},
    global: {
      "@DtSelect": 2,
      "@PowercabMode": 0,
      "@PowercabSelect": 2,
      "@PowercabVoicing": 0,
      "@current_snapshot": 0,
      "@cursor_dsp": 0,
      "@cursor_group": "",
      "@cursor_path": 0,
      "@cursor_position": 0,
      "@guitarinputZ": 8,
      "@guitarpad": 0,
      "@model": "@global_params",
      "@pedalstate": 2,
      "@tempo": 120.0,
      "@topology0": input.topology0,
      "@topology1": 0,
    },
    variax: {
      "@variax_customtuning": false,
      "@variax_lockctrls": 0,
      "@variax_magmode": true,
      "@variax_model": 0,
      "@variax_str1tuning": 0,
      "@variax_str2tuning": 0,
      "@variax_str3tuning": 0,
      "@variax_str4tuning": 0,
      "@variax_str5tuning": 0,
      "@variax_str6tuning": 0,
      "@variax_toneknob": -0.1,
      "@variax_volumeknob": -0.1,
    },
  };

  for (let si = 0; si < snapshotCount; si += 1) {
    const states = snapshotStatesList[si] ?? input.snapshotBlockStates;
    tone[`snapshot${si}`] = buildSnapshot(si, states);
  }

  return {
    schema: "L6Preset",
    version: 6,
    data: {
      device: input.deviceId ?? HLX_DEVICE_ID_STOMP_XL,
      device_version: input.deviceVersion ?? 0,
      meta: {
        application: "HXLinux",
        appversion: input.appVersion ?? 0,
        build_sha: "",
        modifieddate: now,
        name: input.presetName,
      },
      tone,
    },
    meta: {
      original: 0,
      pbn: 0,
      premium: 0,
    },
  };
}

function buildSnapshot(index: number, states: HlxSnapshotBlockStates): Record<string, unknown> {
  const blocks: Record<string, boolean> = {};
  for (const [key, on] of Object.entries(states)) {
    blocks[key] = on;
  }
  return {
    "@custom_name": false,
    "@ledcolor": 0,
    "@name": `SNAPSHOT ${index + 1}`,
    "@pedalstate": 0,
    "@tempo": 120.0,
    "@valid": true,
    blocks: { dsp0: blocks },
  };
}

function flowBlockToHlx(block: HlxFlowBlockExport): Record<string, unknown> {
  const out: Record<string, unknown> = {
    "@model": block.catalogModelId,
    ...block.params,
  };
  if (block.catalogModelId.includes("FlowSplit") || block.catalogModelId.includes("FlowJoin")) {
    out["@enabled"] = block.enabled;
    out["@no_snapshot_bypass"] = false;
    out["@position"] = block.position;
  }
  return out;
}

function fxBlockToHlx(block: HlxFxBlockExport): Record<string, unknown> {
  const out: Record<string, unknown> = {
    "@enabled": block.enabled,
    "@model": block.catalogModelId,
    "@no_snapshot_bypass": false,
    "@path": block.path,
    "@position": block.position,
    "@type": block.blockType,
    ...block.params,
  };
  if (block.stereo !== null) {
    out["@stereo"] = block.stereo;
  }
  if (block.bypassVolume !== null && block.blockType === 1) {
    out["@bypassvolume"] = block.bypassVolume;
  }
  return out;
}

/** Topologie HX Edit (`SABJ` = Split + path A + path B + Join). */
export function computeHlxTopology0(hasPathA: boolean, hasPathB: boolean, hasSplit: boolean): string {
  if (!hasSplit && !hasPathB) return "A";
  let s = "";
  if (hasSplit) s += "S";
  if (hasPathA) s += "A";
  if (hasPathB) s += "B";
  if (hasSplit) s += "J";
  return s || "A";
}

/** Arrondi export `.hlx` : entiers inchangés, flottants à 3 décimales max. */
export function roundHlxExportNumber(n: number): number {
  if (!Number.isFinite(n)) return n;
  if (Number.isInteger(n)) return n;
  return Math.round(n * 1000) / 1000;
}

/** Colonne paire de la grille HX Edit (2, 4, … 18) pour le slot FX `0…7` sur un path. */
export function hlxMatrixColumnForSlotIndex(slotIndex: number): number {
  return 2 + 2 * slotIndex;
}

/** Colonne paire pour une frontière split/merge (`0` = après Input → col 2 ; `1…8` → 4, 6, … 18). */
export function hlxMatrixEvenColForRoutingBoundary(boundary: number): number {
  if (boundary === 0) return 2;
  if (boundary < 1 || boundary > 8) return 2;
  return 4 + 2 * (boundary - 1);
}

/** `@position` dans un `.hlx` à partir du numéro de colonne grille (col 0 = In, 2 = slot 0, 10 = slot 4, …). */
export function hlxPositionFromMatrixColumn(matrixCol: number): number {
  return matrixCol / 2 - 1;
}

/**
 * `@position` d'un bloc FX Kempline (`ki` 0…7 path A, 8…15 path B).
 * Slot `s` occupe la colonne `2·(s+1)` ; position = (col/2) − 1 = `s`.
 * Ex. slot 2 → col 6 → position 2 (Amp Nrm) ; slot 4 → col 10 → position 4 (Comp).
 */
export function hlxPositionForKemplineSlot(ki: number, _gridX?: string): number {
  const slotIndex = ki & 7;
  return hlxPositionFromMatrixColumn(hlxMatrixColumnForSlotIndex(slotIndex));
}
