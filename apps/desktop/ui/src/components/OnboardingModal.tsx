import { useEffect, useState } from "react";
import { getBrowserExtensionStatus, launchBrowserWithExtension, openBrowserExtensionInstallPage } from "../api/browserExtensions";
import type { BrowserExtensionStatus } from "../api/types";
import { errorMessage } from "../lib/format";

const STORAGE_KEY = "fetchx_onboarded";

const FIREFOX_FAMILY = new Set([
  "Mozilla Firefox",
  "LibreWolf",
  "Waterfox",
  "Floorp",
]);

interface Props {
  openAddDialog: (url?: string) => void;
}

export function OnboardingModal({ openAddDialog }: Props) {
  const [visible, setVisible] = useState(
    () => localStorage.getItem(STORAGE_KEY) !== "1",
  );
  const [step, setStep] = useState(0);
  const [browserStatus, setBrowserStatus] = useState<BrowserExtensionStatus | null>(null);
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const [url, setUrl] = useState("");

  useEffect(() => {
    if (!visible) return;
    getBrowserExtensionStatus()
      .then(setBrowserStatus)
      .catch(() => { /* non-critical */ });
  }, [visible]);

  if (!visible) return null;

  const detectedBrowsers = browserStatus?.browsers.filter((b) => b.detected) ?? [];

  const finish = () => {
    localStorage.setItem(STORAGE_KEY, "1");
    setVisible(false);
  };

  const handleInstall = async (browser: string) => {
    setBusy((prev) => new Set([...prev, browser]));
    try {
      if (FIREFOX_FAMILY.has(browser)) {
        await openBrowserExtensionInstallPage(browser);
      } else {
        await launchBrowserWithExtension(browser);
      }
    } catch (cause) {
      /* non-critical — user can install manually */
      void errorMessage(cause);
    } finally {
      setBusy((prev) => {
        const next = new Set(prev);
        next.delete(browser);
        return next;
      });
    }
  };

  const handleAddDownload = () => {
    if (!url.trim()) return;
    openAddDialog(url.trim());
    finish();
  };

  return (
    <div
      className="dialog-backdrop onboarding-backdrop"
      role="dialog"
      aria-modal="true"
      aria-label="Welcome to FetchX"
    >
      <div className="dialog onboarding-card">
        {/* Steps */}
        <div className="onboarding-body">
          {step === 0 && <WelcomeStep />}
          {step === 1 && (
            <ExtensionStep
              browsers={detectedBrowsers}
              busy={busy}
              onInstall={handleInstall}
            />
          )}
          {step === 2 && (
            <ReadyStep
              url={url}
              onUrlChange={setUrl}
              onAddDownload={handleAddDownload}
            />
          )}
        </div>

        {/* Navigation */}
        <div className="onboarding-footer">
          <div className="onboarding-dots" aria-label="Step indicator">
            {[0, 1, 2].map((i) => (
              <span
                key={i}
                className={`onboarding-dot${i === step ? " onboarding-dot--active" : ""}`}
                aria-current={i === step ? "step" : undefined}
              />
            ))}
          </div>

          <div className="dialog-actions">
            {step > 0 && (
              <button
                className="button button-secondary"
                onClick={() => setStep((s) => s - 1)}
              >
                Back
              </button>
            )}

            {step === 0 && (
              <button
                className="button button-primary"
                onClick={() => setStep(1)}
              >
                Get started
              </button>
            )}

            {step === 1 && (
              <>
                <button
                  className="button button-ghost onboarding-skip"
                  onClick={() => setStep(2)}
                >
                  I'll do this later
                </button>
                <button
                  className="button button-primary"
                  onClick={() => setStep(2)}
                >
                  Next
                </button>
              </>
            )}

            {step === 2 && (
              <button
                className="button button-primary"
                onClick={finish}
              >
                Go to downloads
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Step components ──────────────────────────────────────────────────────────

function WelcomeStep() {
  return (
    <div className="onboarding-step">
      <div className="onboarding-logo-wrap" aria-hidden="true">
        <svg
          className="onboarding-logo"
          viewBox="0 0 48 48"
          fill="none"
          xmlns="http://www.w3.org/2000/svg"
          aria-hidden="true"
        >
          <rect width="48" height="48" rx="12" fill="var(--accent-subtle)" />
          <path
            d="M24 10v20M14 22l10 10 10-10"
            stroke="var(--accent)"
            strokeWidth="2.5"
            strokeLinecap="round"
            strokeLinejoin="round"
          />
          <rect x="12" y="34" width="24" height="3" rx="1.5" fill="var(--accent)" opacity="0.6" />
        </svg>
      </div>
      <h2 className="onboarding-headline">Welcome to FetchX</h2>
      <p className="onboarding-body-text">
        A fast, modern download manager for Windows. Let's get you set up in 3 steps.
      </p>
    </div>
  );
}

interface ExtensionStepProps {
  browsers: Array<{ browser: string; detected: boolean }>;
  busy: Set<string>;
  onInstall: (browser: string) => void;
}

function ExtensionStep({ browsers, busy, onInstall }: ExtensionStepProps) {
  return (
    <div className="onboarding-step">
      <h2 className="onboarding-headline">Connect your browser</h2>
      <p className="onboarding-body-text">
        The FetchX extension intercepts downloads and sends media streams straight to FetchX.
      </p>
      {browsers.length > 0 ? (
        <div className="onboarding-browser-list">
          {browsers.map((b) => (
            <div key={b.browser} className="onboarding-browser-row">
              <span className="onboarding-browser-name">{b.browser}</span>
              <button
                className="button button-secondary button-xs"
                onClick={() => onInstall(b.browser)}
                disabled={busy.has(b.browser)}
              >
                {busy.has(b.browser) ? "Installing…" : "Install"}
              </button>
            </div>
          ))}
        </div>
      ) : (
        <p className="onboarding-no-browsers">
          No supported browsers detected. You can install the extension later from Settings.
        </p>
      )}
    </div>
  );
}

interface ReadyStepProps {
  url: string;
  onUrlChange: (v: string) => void;
  onAddDownload: () => void;
}

function ReadyStep({ url, onUrlChange, onAddDownload }: ReadyStepProps) {
  return (
    <div className="onboarding-step">
      <h2 className="onboarding-headline">You're ready</h2>
      <p className="onboarding-body-text">
        Paste a URL below to add your first download, or drag a URL into the app at any time.
      </p>
      <div className="onboarding-url-row">
        <input
          className="field-input onboarding-url-input"
          type="url"
          placeholder="https://example.com/file.zip"
          value={url}
          onChange={(e) => onUrlChange(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") onAddDownload(); }}
          autoFocus
        />
        <button
          className="button button-primary"
          onClick={onAddDownload}
          disabled={!url.trim()}
        >
          Add download
        </button>
      </div>
    </div>
  );
}
