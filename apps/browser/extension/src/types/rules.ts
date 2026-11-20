export interface InterceptRule {
  id: string;
  enabled: boolean;
  name: string;
  match:
    | { kind: "domain"; domain: string }
    | { kind: "url_pattern"; pattern: string }
    | { kind: "file_extension"; ext: string };
  action: "always_intercept" | "never_intercept";
}

/** Convert a glob pattern (supporting * and ?) to a RegExp. */
export function globToRegex(pattern: string): RegExp {
  const escaped = pattern.replace(/[.+^${}()|[\]\\]/g, "\\$&");
  const regexStr = escaped.replace(/\*/g, ".*").replace(/\?/g, ".");
  return new RegExp(`^${regexStr}$`, "i");
}

/**
 * Evaluate all enabled rules against a URL in order.
 * Returns the action of the first matching enabled rule, or null if none match.
 */
export function evaluateInterceptRules(
  url: string,
  rules: InterceptRule[],
): "always" | "never" | null {
  let hostname: string;
  let pathname: string;
  try {
    const parsed = new URL(url);
    hostname = parsed.hostname.toLowerCase().replace(/\.$/, "");
    pathname = parsed.pathname;
  } catch {
    return null;
  }

  for (const rule of rules) {
    if (!rule.enabled) continue;
    if (matchesRule(url, rule, hostname, pathname)) {
      return rule.action === "always_intercept" ? "always" : "never";
    }
  }
  return null;
}

/** Returns true when the given URL matches the rule's match criteria. */
export function matchesRule(url: string, rule: InterceptRule): boolean;
export function matchesRule(
  url: string,
  rule: InterceptRule,
  hostname?: string,
  pathname?: string,
): boolean;
export function matchesRule(
  url: string,
  rule: InterceptRule,
  hostnameArg?: string,
  pathnameArg?: string,
): boolean {
  let hostname: string = hostnameArg ?? "";
  let pathname: string = pathnameArg ?? "";
  if (!hostnameArg || !pathnameArg) {
    try {
      const parsed = new URL(url);
      hostname = parsed.hostname.toLowerCase().replace(/\.$/, "");
      pathname = parsed.pathname;
    } catch {
      return false;
    }
  }

  const { match } = rule;
  switch (match.kind) {
    case "domain": {
      const domain = match.domain.trim().toLowerCase().replace(/^\.+|\.+$/g, "");
      return domain !== "" && (hostname === domain || hostname.endsWith(`.${domain}`));
    }
    case "url_pattern": {
      const regex = globToRegex(match.pattern);
      return regex.test(url);
    }
    case "file_extension": {
      const ext = match.ext.trim().toLowerCase().replace(/^\.+/, "");
      if (!ext) return false;
      const lastSegment = pathname.split("/").pop() ?? "";
      const dotIndex = lastSegment.lastIndexOf(".");
      if (dotIndex < 0) return false;
      const fileExt = lastSegment.slice(dotIndex + 1).toLowerCase();
      return fileExt === ext;
    }
  }
}
