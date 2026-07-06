import { createRoot } from "react-dom/client";
import { Component, type ErrorInfo, type ReactNode } from "react";
import { BrowserRouter } from "react-router-dom";
import "./index.css";
import App from "./App";
import { SystemActionsProvider } from "./contexts/SystemActions";
import { I18nProvider } from "./i18n";
import { exposePluginSDK } from "./plugins";
import { ThemeProvider } from "./themes";
import { HERMES_BASE_PATH } from "./lib/api";

// Expose the plugin SDK before rendering so plugins loaded via <script>
// can access React, components, etc. immediately.
exposePluginSDK();

/**
 * Top-level ErrorBoundary — prevents a render error in any page from
 * unmounting the whole dashboard (blank screen). (iter-128 — closes the
 * ponytail-audit gap "No ErrorBoundary anywhere in the SPA".)
 *
 * Renders a minimal error card with the stack trace + a "Reload" button.
 * Intentionally not translated — if i18n itself crashed, we still want
 * a recoverable error surface.
 */
class TopLevelErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state: { error: Error | null } = { error: null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    // eslint-disable-next-line no-console
    console.error("[operant-dashboard] Uncaught render error:", error, info);
  }

  handleReload = () => {
    this.setState({ error: null });
    // Hard reload — the broken component tree can't be safely recovered
    // in-place without knowing which subtree crashed.
    window.location.reload();
  };

  render() {
    if (!this.state.error) return this.props.children;
    const err = this.state.error;
    return (
      <div
        style={{
          position: "fixed",
          inset: 0,
          display: "flex",
          alignItems: "center",
          justifyContent: "center",
          background: "#0b0d12",
          color: "#e6e8ee",
          fontFamily:
            "ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace",
          padding: "2rem",
        }}
      >
        <div
          style={{
            maxWidth: "640px",
            width: "100%",
            background: "#13161d",
            border: "1px solid #2a2f3a",
            borderRadius: "8px",
            padding: "1.5rem",
          }}
        >
          <h1 style={{ margin: "0 0 0.5rem", fontSize: "1.1rem" }}>
            Dashboard render error
          </h1>
          <p style={{ margin: "0 0 1rem", color: "#9aa0ad", fontSize: "0.875rem" }}>
            Something crashed while rendering the dashboard. Reloading usually
            fixes it; if the error persists, check the browser console.
          </p>
          <pre
            style={{
              margin: "0 0 1rem",
              padding: "0.75rem",
              background: "#0b0d12",
              border: "1px solid #2a2f3a",
              borderRadius: "4px",
              fontSize: "0.75rem",
              overflow: "auto",
              maxHeight: "240px",
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
            }}
          >
            {err.name}: {err.message}
            {err.stack ? `\n\n${err.stack}` : ""}
          </pre>
          <button
            type="button"
            onClick={this.handleReload}
            style={{
              background: "#6366f1",
              color: "#fff",
              border: "none",
              borderRadius: "4px",
              padding: "0.5rem 1rem",
              fontSize: "0.875rem",
              cursor: "pointer",
            }}
          >
            Reload dashboard
          </button>
        </div>
      </div>
    );
  }
}

createRoot(document.getElementById("root")!).render(
  <TopLevelErrorBoundary>
    <BrowserRouter basename={HERMES_BASE_PATH || undefined}>
      <I18nProvider>
        <ThemeProvider>
          <SystemActionsProvider>
            <App />
          </SystemActionsProvider>
        </ThemeProvider>
      </I18nProvider>
    </BrowserRouter>
  </TopLevelErrorBoundary>,
);
