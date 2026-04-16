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
    if (isChunkLoadError(error)) {
      return { chunkError: true };
    }
    return null;
  }

  componentDidCatch(error: Error): void {
    if (!isChunkLoadError(error)) {
      throw error;
    }

    // First attempt: auto-reload (guard prevents infinite loop)
    if (!sessionStorage.getItem(RELOAD_KEY)) {
      sessionStorage.setItem(RELOAD_KEY, "1");
      window.location.reload();
    }
    // Second attempt: guard is set, so we show the fallback UI
  }

  render(): ReactNode {
    if (this.state.chunkError && sessionStorage.getItem(RELOAD_KEY)) {
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
          <p style={{ fontSize: "1.125rem", marginBottom: "1.5rem" }}>
            A new version has been deployed.
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
