/**
 * Point d’entrée unique pour les rafraîchissements UI déclenchés par le hardware (ou le soft-sync).
 *
 * - **Immédiat** (`runImmediate`) : grille, picker — léger.
 * - **Après geste HW** (`scheduleAfterHwGesture`) : params lourds ~200 ms après molette / knob.
 * - **Soft-sync idle** (`runParamsSyncWhenIdle`) : uniquement si aucun geste ni job lourd en cours.
 *
 * Un seul timer settle ; un seul job lourd à la fois (pas d’exécutions parallèles).
 */

export const HW_UI_SETTLE_MS = 200;

export type HwUiChannel = "params" | "grid" | "picker";

export type HwUiHeavyJob = () => void | Promise<void>;

export type HwUiImmediateJob = () => void;

export type HwUiRefreshHooks = {
  setParamsBrowsingMode?: (browsing: boolean) => void;
};

type PendingHeavy = { channel: HwUiChannel; job: HwUiHeavyJob };

class HwUiRefreshCoordinator {
  private hooks: HwUiRefreshHooks = {};
  private settleTimer: ReturnType<typeof setTimeout> | null = null;
  private settleSeq = 0;
  private pendingHeavy: PendingHeavy | null = null;
  private queuedHeavy: PendingHeavy | null = null;
  private heavyInFlight = false;
  private _blockSyntheticParamsLoad = false;

  configure(hooks: HwUiRefreshHooks): void {
    this.hooks = hooks;
  }

  get blockSyntheticParamsLoad(): boolean {
    return this._blockSyntheticParamsLoad;
  }

  get gestureInProgress(): boolean {
    return this._blockSyntheticParamsLoad || this.settleTimer !== null || this.heavyInFlight;
  }

  bumpFromBus(): void {
    this._blockSyntheticParamsLoad = true;
    this.hooks.setParamsBrowsingMode?.(true);
    // Ne pas clearTimeout ici : seul armSettleTimer gère le timer (évite annuler sans re-créer).
  }

  runImmediate(_channel: HwUiChannel, job: HwUiImmediateJob): void {
    job();
  }

  private clearSettleTimer(): void {
    if (this.settleTimer !== null) {
      clearTimeout(this.settleTimer);
      this.settleTimer = null;
    }
  }

  private armSettleTimer(): void {
    this.clearSettleTimer();
    const seq = ++this.settleSeq;
    this.settleTimer = setTimeout(() => {
      this.settleTimer = null;
      if (seq !== this.settleSeq) return;
      this._blockSyntheticParamsLoad = false;
      this.hooks.setParamsBrowsingMode?.(false);
      const pending = this.pendingHeavy;
      this.pendingHeavy = null;
      if (!pending) return;
      void this.runHeavySerial(pending);
    }, HW_UI_SETTLE_MS);
  }

  /** Un job lourd à la fois ; le suivant attend la fin du précédent. */
  private heavyDebugEnabled(): boolean {
    try {
      return localStorage.getItem("models_debug_heavy_ui") === "1";
    } catch {
      return false;
    }
  }

  private async runHeavySerial(entry: PendingHeavy): Promise<void> {
    if (this.heavyInFlight) {
      this.queuedHeavy = entry;
      return;
    }
    this.heavyInFlight = true;
    const t0 = this.heavyDebugEnabled() ? performance.now() : 0;
    try {
      await entry.job();
    } catch (e) {
      console.warn(`[hwUiRefresh] ${entry.channel} heavy job failed`, e);
    } finally {
      if (this.heavyDebugEnabled()) {
        const ms = Math.round(performance.now() - t0);
        console.log(`[hwUiRefresh] ${entry.channel} heavy done ${ms}ms queued=${this.queuedHeavy !== null}`);
      }
      this.heavyInFlight = false;
      const next = this.queuedHeavy;
      this.queuedHeavy = null;
      if (next) {
        await this.runHeavySerial(next);
      }
    }
  }

  scheduleAfterHwGesture(channel: HwUiChannel, job: HwUiHeavyJob): void {
    this.bumpFromBus();
    this.pendingHeavy = { channel, job };
    this.armSettleTimer();
  }

  runParamsSyncWhenIdle(channel: HwUiChannel, job: HwUiHeavyJob): void {
    if (this.gestureInProgress) {
      return;
    }
    void this.runHeavySerial({ channel, job });
  }
}

export const hwUi = new HwUiRefreshCoordinator();
