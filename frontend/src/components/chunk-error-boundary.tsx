import { Component, type ReactNode } from "react";

const RELOAD_KEY = "nyxid_chunk_reload";

function isChunkLoadError(error: Error): boolean {
  const msg = error.message || "";
  return (
    msg.includes("Failed to fetch dynamically imported module") ||
    msg.includes("Importing a module script failed") ||
    msg.includes("ChunkLoadError") ||
    /Loading chunk .+ failed/.test(msg)
  );
}

interface Props {
  children: ReactNode;
}

interface State {
  chunkError: boolean;
}

export class ChunkErrorBoundary extends Component<Props, State> {
  constructor(props: Props) {
    super(props);
    this.state = { chunkError: false };
  }

  static getDerivedStateFromError(error: Error): State | null {
    if (!isChunkLoadError(error)) {
      return null;
    }

    // First chunk error: auto-reload to pick up new assets.
    // The sessionStorage guard prevents an infinite reload loop.
    if (!sessionStorage.getItem(RELOAD_KEY)) {
      sessionStorage.setItem(RELOAD_KEY, "1");
      window.location.reload();
      // Return chunkError so React doesn't try to render stale children
      // while the reload is in progress.
      return { chunkError: true };
    }

    // Reload already happened and chunks still fail — show fallback UI.
    return { chunkError: true };
  }

  componentDidCatch(error: Error): void {
    if (!isChunkLoadError(error)) {
      throw error;
    }
  }

  render(): ReactNode {
    if (this.state.chunkError) {
      return (
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            alignItems: "center",
            justifyContent: "center",
            minHeight: "100vh",
            background: "#09090b",
            color: "#e4e4e7",
            fontFamily: "system-ui, sans-serif",
            padding: "1rem",
            textAlign: "center",
          }}
        >
          <p style={{ fontSize: "1.125rem", marginBottom: "0.5rem" }}>
            A new version has been deployed.
          </p>
          <p
            style={{
              fontSize: "0.875rem",
              color: "#a1a1aa",
              marginBottom: "1.5rem",
              maxWidth: "24rem",
            }}
          >
            Your browser has cached an older version of the app. Please reload
            to get the latest update.
          </p>
          <button
            type="button"
            onClick={() => {
              sessionStorage.removeItem(RELOAD_KEY);
              window.location.reload();
            }}
            style={{
              padding: "0.625rem 1.25rem",
              fontSize: "0.875rem",
              fontWeight: 500,
              color: "#09090b",
              background: "#e4e4e7",
              border: "none",
              borderRadius: "0.375rem",
              cursor: "pointer",
            }}
          >
            Reload page
          </button>
        </div>
      );
    }

    return this.props.children;
  }
}
