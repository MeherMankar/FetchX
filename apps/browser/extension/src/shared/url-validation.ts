/**
 * Validates a URL client-side for fast UX feedback.
 * The native host and desktop application perform the authoritative check.
 */
export function isHttpUrl(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
}

/**
 * Returns true if the URL contains embedded credentials.
 * Mirrors the server-side check.
 */
export function hasEmbeddedCredentials(url: string): boolean {
  try {
    const parsed = new URL(url);
    return parsed.username !== "" || parsed.password !== "";
  } catch {
    return false;
  }
}
