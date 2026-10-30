import { describe, expect, it } from "vitest";

import {
  claimDownloadEvent,
  filenameFromBrowserPath,
  handoffBrowserDownload,
  isBlockedDomain,
  resolveDownloadFilename,
  submitDownloadWithResolvedFilename,
  type DownloadHandoffPort,
} from "./download-interception";

function handoffPort(
  events: string[],
  overrides: Partial<DownloadHandoffPort> = {},
): DownloadHandoffPort {
  return {
    pause: async (id) => {
      events.push(`pause:${id}`);
    },
    resume: async (id) => {
      events.push(`resume:${id}`);
    },
    cancel: async (id) => {
      events.push(`cancel:${id}`);
    },
    submit: async () => {
      events.push("submit");
      return { kind: "accepted", downloadId: "fetchx-1" };
    },
    ...overrides,
  };
}

describe("browser download handoff", () => {
  it("pauses before submitting and cancels only after acceptance", async () => {
    const events: string[] = [];

    const result = await handoffBrowserDownload(7, handoffPort(events));

    expect(result).toEqual({
      kind: "accepted",
      downloadId: "fetchx-1",
      browserDownloadCancelled: true,
    });
    expect(events).toEqual(["pause:7", "submit", "cancel:7"]);
  });

  it("resumes the browser download after a definite rejection", async () => {
    const events: string[] = [];
    const result = await handoffBrowserDownload(
      7,
      handoffPort(events, {
        submit: async () => {
          events.push("submit");
          return { kind: "rejected", message: "target rejected request" };
        },
      }),
    );

    expect(result).toEqual({
      kind: "rejected",
      message: "target rejected request",
      browserDownloadResumed: true,
    });
    expect(events).toEqual(["pause:7", "submit", "resume:7"]);
  });

  it("leaves the browser download paused after an ambiguous result", async () => {
    const events: string[] = [];
    const result = await handoffBrowserDownload(
      7,
      handoffPort(events, {
        submit: async () => {
          events.push("submit");
          return { kind: "unknown", message: "confirmation missing" };
        },
      }),
    );

    expect(result).toEqual({ kind: "unknown", message: "confirmation missing" });
    expect(events).toEqual(["pause:7", "submit"]);
  });

  it("does not submit if the browser could not pause its download", async () => {
    const events: string[] = [];
    const result = await handoffBrowserDownload(
      7,
      handoffPort(events, {
        pause: async () => {
          events.push("pause:7");
          throw new Error("already completed");
        },
      }),
    );

    expect(result).toEqual({ kind: "not_paused" });
    expect(events).toEqual(["pause:7"]);
  });

  it("keeps the original paused when cancellation fails after acceptance", async () => {
    const events: string[] = [];
    const result = await handoffBrowserDownload(
      7,
      handoffPort(events, {
        cancel: async () => {
          events.push("cancel:7");
          throw new Error("download already completed");
        },
      }),
    );

    expect(result).toEqual({
      kind: "accepted",
      downloadId: "fetchx-1",
      browserDownloadCancelled: false,
    });
    expect(events).toEqual(["pause:7", "submit", "cancel:7"]);
  });
});

describe("browser download metadata", () => {
  it("forwards only the basename, never the browser's absolute path", () => {
    expect(filenameFromBrowserPath("C:\\Users\\Me\\Downloads\\report.zip"))
      .toBe("report.zip");
    expect(filenameFromBrowserPath("/home/user/Downloads/report.zip"))
      .toBe("report.zip");
    expect(filenameFromBrowserPath(undefined)).toBeUndefined();
  });

  it("claims a browser download ID only once", async () => {
    let stored: unknown = [];
    const saved: number[][] = [];
    const claims = new Set<number>();
    const store = {
      load: async () => stored,
      save: async (ids: number[]) => {
        stored = ids;
        saved.push(ids);
      },
    };

    expect(await claimDownloadEvent(17, claims, store, 512)).toBe(true);
    expect(await claimDownloadEvent(17, claims, store, 512)).toBe(false);
    expect(saved).toEqual([[17]]);
  });

  it("does not submit a concurrent duplicate browser event", async () => {
    let releaseLoad: (value: unknown) => void = () => undefined;
    const loading = new Promise<unknown>((resolve) => {
      releaseLoad = resolve;
    });
    const claims = new Set<number>();
    const store = {
      load: async () => loading,
      save: async () => undefined,
    };

    const first = claimDownloadEvent(17, claims, store, 512);
    await Promise.resolve();
    await expect(claimDownloadEvent(17, claims, store, 512)).resolves.toBe(false);
    releaseLoad([]);
    await expect(first).resolves.toBe(true);
  });

  it("uses only the final path component for untrusted browser paths", () => {
    expect(filenameFromBrowserPath("../../secret.txt")).toBe("secret.txt");
    expect(filenameFromBrowserPath(String.raw`..\..\secret.txt`)).toBe("secret.txt");
    expect(filenameFromBrowserPath(String.raw`C:\Windows\file.txt`)).toBe("file.txt");
    expect(filenameFromBrowserPath("report?.txt")).toBe("report?.txt");
  });

  it("matches blocked domains only at a domain boundary", () => {
    expect(isBlockedDomain("cdn.example.com", ["example.com"])).toBe(true);
    expect(isBlockedDomain("example.com", ["example.com"])).toBe(true);
    expect(isBlockedDomain("notexample.com", ["example.com"])).toBe(false);
  });

  it("accepts a Firefox download item through the shared handoff port", async () => {
    const item = {
      id: 29,
      url: "https://example.test/fixture.bin",
      filename: String.raw`C:\Users\Me\Downloads\fixture.bin`,
      fileSize: 174,
    };
    const events: string[] = [];
    let submittedFilename: string | undefined;

    const result = await handoffBrowserDownload(
      item.id,
      handoffPort(events, {
        submit: async () => {
          submittedFilename = filenameFromBrowserPath(item.filename);
          events.push(`submit:${submittedFilename}`);
          return { kind: "accepted", downloadId: "firefox-fetchx-1" };
        },
      }),
    );

    expect(result.kind).toBe("accepted");
    expect(submittedFilename).toBe("fixture.bin");
    expect(events).toEqual(["pause:29", "submit:fixture.bin", "cancel:29"]);
  });

  it("waits for Firefox's late filename metadata and removes the event listener", async () => {
    let listener: ((delta: { id: number; filename?: { current?: string } }) => void) | undefined;
    let removed = false;
    const port = {
      search: async () => [{ filename: "" }],
      addChangedListener: (callback: (delta: { id: number; filename?: { current?: string } }) => void) => {
        listener = callback;
      },
      removeChangedListener: (callback: (delta: { id: number; filename?: { current?: string } }) => void) => {
        removed = callback === listener;
      },
    };

    const filename = resolveDownloadFilename(31, "", port, 100);
    await Promise.resolve();
    listener?.({ id: 31, filename: { current: String.raw`C:\Downloads\firefox-test.zip` } });

    await expect(filename).resolves.toBe(String.raw`C:\Downloads\firefox-test.zip`);
    expect(removed).toBe(true);
  });

  it("resumes Firefox's download if metadata lookup fails before RPC submission", async () => {
    const events: string[] = [];
    const result = await handoffBrowserDownload(
      32,
      handoffPort(events, {
        submit: () => submitDownloadWithResolvedFilename(
          {
            id: 32,
            url: "https://example.test/fixture.bin",
            filename: "",
          },
          {
            search: async () => {
              throw new Error("downloads.search unavailable");
            },
            addChangedListener: () => undefined,
            removeChangedListener: () => undefined,
          },
          async () => {
            events.push("rpc");
            return { kind: "accepted", downloadId: "unexpected" };
          },
        ),
      }),
    );

    expect(result.kind).toBe("rejected");
    expect(events).toEqual(["pause:32", "resume:32"]);
  });
});

describe("browser URL validation", () => {
  it.each([
    "magnet:?xt=urn:btih:example",
    "file:///C:/Windows/win.ini",
    "javascript:alert(1)",
    "data:text/plain,test",
    "chrome://settings/",
    "chrome-extension://id/page.html",
    "about:config",
    "moz-extension://id/page.html",
  ])(
    "rejects %s",
    async (url) => {
      const { isHttpUrl } = await import("../shared/url-validation");
      expect(isHttpUrl(url)).toBe(false);
    },
  );
});
