import type { LoginCodeStatus } from "../../lib/api/loginCodeApi";

type Api = {
  status(id: string): Promise<LoginCodeStatus>;
  cancel(id: string): Promise<void>;
  revoke(id: string): Promise<void>;
};
export type LoginCodeMonitorState = { status: LoginCodeStatus | null; pending: boolean; error: boolean };

export class LoginCodeMonitor {
  private generation = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private state: LoginCodeMonitorState = { status: null, pending: false, error: false };
  private stopped = true;
  constructor(private api: Api, private id: string,
    private notify: (state: LoginCodeMonitorState) => void, private interval = 5000) {}

  start() { this.stopped = false; void this.refresh(this.generation); }
  stop() { this.stopped = true; this.generation++; this.clearTimer(); }
  private clearTimer() { if (this.timer !== null) clearTimeout(this.timer); this.timer = null; }
  private emit(patch: Partial<LoginCodeMonitorState>) { this.state = { ...this.state, ...patch }; this.notify(this.state); }
  private async refresh(generation: number) {
    try {
      const status = await this.api.status(this.id);
      if (this.stopped || generation !== this.generation) return;
      this.emit({ status, error: false, pending: false });
      if (status.status !== "pending") return;
    } catch {
      if (this.stopped || generation !== this.generation) return;
      this.emit({ error: true, pending: false });
    }
    if (!this.stopped && generation === this.generation) {
      this.timer = setTimeout(() => { this.timer = null; void this.refresh(generation); }, this.interval);
    }
  }
  async act(action: "cancel" | "revoke") {
    if (this.stopped || this.state.pending) return;
    this.clearTimer();
    const generation = ++this.generation;
    this.emit({ pending: true, error: false });
    try {
      await this.api[action](this.id);
      if (this.stopped || generation !== this.generation) return;
      await this.refresh(generation);
    } catch {
      if (!this.stopped && generation === this.generation) {
        this.emit({ pending: false, error: true });
        this.timer = setTimeout(() => { this.timer = null; void this.refresh(generation); }, this.interval);
      }
    }
  }
}
