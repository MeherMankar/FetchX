// Other disconnects may follow a request that was accepted but not confirmed.
const MISSING_NATIVE_HOST_PATTERNS = [
  /^no such native application\b/i,
  /^specified native messaging host not found\b/i,
  /^native messaging host not found\b/i,
];

export function isNativeHostUnavailableError(message: string | undefined): boolean {
  return message !== undefined
    && MISSING_NATIVE_HOST_PATTERNS.some((pattern) => pattern.test(message.trim()));
}
