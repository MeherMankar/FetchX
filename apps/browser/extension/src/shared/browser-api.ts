import { selectBrowserApi } from "./select-browser-api";

export const browserApi = selectBrowserApi(
  typeof browser === "undefined" ? undefined : browser,
  chrome,
);

export type NativePort =
  | ReturnType<typeof chrome.runtime.connectNative>
  | ReturnType<typeof browser.runtime.connectNative>;

export function connectNative(hostName: string): NativePort {
  return typeof browser === "undefined"
    ? chrome.runtime.connectNative(hostName)
    : browser.runtime.connectNative(hostName);
}

export function sendRuntimeMessage<T>(message: unknown): Promise<T> {
  return typeof browser === "undefined"
    ? chrome.runtime.sendMessage(message) as Promise<T>
    : browser.runtime.sendMessage(message) as Promise<T>;
}
