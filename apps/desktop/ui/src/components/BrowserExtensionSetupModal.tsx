import { useEffect, useState } from "react";
import {
  getBrowserExtensionStatus,
  launchBrowserWithExtension,
  openBrowserExtensionInstallPage,
} from "../api/browserExtensions";
import type { BrowserRegistrationResult, BrowserExtensionStatus } from "../api/types";
import { errorMessage } from "../lib/format";

const FIREFOX_FAMILY = new Set([
  "Mozilla Firefox",
  "LibreWolf",
  "Waterfox",
  "Floorp",
]);

const STORAGE_KEY = "fetchx_extension_setup_done";

type InstallResult = { browser: string; ok: boolean; error?: string };

export function BrowserExtensionSetupModal() {
  const [status, setStatus] = useState<BrowserExtensionStatus | null>(null);
  const [dismissed, setDismissed] = useState(() => localStorage.getItem(STORAGE_KEY) === "1");
  const [mode, setMode] = useState<"prompt" | "choose">("prompt");
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [results, setResults] = useState<InstallResult[]>([]);
  const [done, setDone] = useState(false);

  useEffect(() => {
    if (dismissed) return;
    getBrowserExtensionStatus()
      .then((s) => setStatus(s))
      .catch(() => { /* non-critical; modal silently does not appear */ });
  }, [dismissed]);

  const detectedBrowsers = status?.browsers.filter((b) => b.detected) ?? [];

  if (dismissed || detectedBrowsers.length === 0 || !status) {
    return null;
  }

  if (done && results.length > 0) {
    const allOk = results.every((r) => r.ok);
    return (
      <div className="modal-backdrop" role="dialog" aria-modal="true" aria-labelledby="ext-setup-done-heading">
        <div className="modal">
          <h2 id="ext-setup-done-heading" className="modal-title">Browser extension setup</h2>
          <div className="ext-setup-results">
            {results.map((r) => (
              <div key={r.browser} className={`ext-setup-result ${r.ok ? "ext-setup-result-ok" : "ext-setup-result-err"}`}>
                <span>{r.ok ? "✓" : "✗"}</span>
                <span>{r.browser}</span>
                {r.error && <span className="ext-setup-result-error">{r.error}</span>}
              </div>
            ))}
          </div>
          {!allOk && (
            <p className="settings-hint">
              Some browsers could not be launched automatically. Use Settings → Browser Extensions to retry.
            </p>
          )}
          <div className="modal-actions">
            <button
              className="button button-primary"
              onClick={() => {
                localStorage.setItem(STORAGE_KEY, "1");
                setDismissed(true);
              }}
            >
              Done
            </button>
          </div>
        </div>
      </div>
    );
  }

  const installBrowsers = async (browsers: BrowserRegistrationResult[]) => {
    const installResults: InstallResult[] = [];
    for (const b of browsers) {
      setBusy((prev) => new Set([...prev, b.browser]));
      try {
        if (FIREFOX_FAMILY.has(b.browser)) {
          await openBrowserExtensionInstallPage(b.browser);
        } else {
          await launchBrowserWithExtension(b.browser);
        }
        installResults.push({ browser: b.browser, ok: true });
      } catch (cause) {
        installResults.push({ browser: b.browser, ok: false, error: errorMessage(cause) });
      } finally {
        setBusy((prev) => {
          const next = new Set(prev);
          next.delete(b.browser);
          return next;
        });
      }
    }
    setResults(installResults);
    localStorage.setItem(STORAGE_KEY, "1");
    setDone(true);
  };

  const handleInstallAll = () => void installBrowsers(detectedBrowsers);

  const handleInstallChosen = () => {
    const browsers = detectedBrowsers.filter((b) => checked.has(b.browser));
    if (browsers.length === 0) return;
    void installBrowsers(browsers);
  };

  const handleSkip = () => {
    localStorage.setItem(STORAGE_KEY, "1");
    setDismissed(true);
  };

  const toggleChecked = (browser: string) => {
    setChecked((prev) => {
      const next = new Set(prev);
      if (next.has(browser)) next.delete(browser);
      else next.add(browser);
      return next;
    });
  };

  const isBusy = busy.size > 0;

  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true" aria-labelledby="ext-setup-heading">
      <div className="modal">
        <h2 id="ext-setup-heading" className="modal-title">Browser extension setup</h2>
        <p className="modal-body">
          FetchX detected {detectedBrowsers.length} browser{detectedBrowsers.length !== 1 ? "s" : ""}.
          Install the extension to capture downloads?
        </p>

        {mode === "choose" && (
          <div className="ext-setup-checklist" role="group" aria-label="Select browsers">
            {detectedBrowsers.map((b) => (
              <label key={b.browser} className="ext-setup-check-label">
                <input
                  type="checkbox"
                  checked={checked.has(b.browser)}
                  onChange={() => toggleChecked(b.browser)}
                  disabled={isBusy}
                />
                <span>{b.browser}</span>
                {busy.has(b.browser) && <span className="ext-setup-spinner" aria-label="Installing…">⏳</span>}
              </label>
            ))}
          </div>
        )}

        {mode === "prompt" && (
          <div className="ext-setup-browser-list">
            {detectedBrowsers.map((b) => (
              <div key={b.browser} className="ext-setup-browser-item">
                <span>{b.browser}</span>
                {busy.has(b.browser) && <span className="ext-setup-spinner" aria-label="Installing…">⏳</span>}
              </div>
            ))}
          </div>
        )}

        <div className="modal-actions">
          {mode === "prompt" ? (
            <>
              <button
                className="button button-primary"
                onClick={handleInstallAll}
                disabled={isBusy}
              >
                Install in all browsers
              </button>
              <button
                className="button button-secondary"
                onClick={() => {
                  setChecked(new Set(detectedBrowsers.map((b) => b.browser)));
                  setMode("choose");
                }}
                disabled={isBusy}
              >
                Choose browsers
              </button>
              <button
                className="button button-ghost"
                onClick={handleSkip}
                disabled={isBusy}
              >
                Skip
              </button>
            </>
          ) : (
            <>
              <button
                className="button button-primary"
                onClick={handleInstallChosen}
                disabled={isBusy || checked.size === 0}
              >
                Confirm
              </button>
              <button
                className="button button-secondary"
                onClick={() => setMode("prompt")}
                disabled={isBusy}
              >
                Back
              </button>
              <button
                className="button button-ghost"
                onClick={handleSkip}
                disabled={isBusy}
              >
                Skip
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
