// Which URLs model output may use (spec, Rendering Untrusted Output): links go only to `http`,
// `https`, and `mailto`; images render inline only from `blob:` and `data:image/` URLs, and any
// other image becomes a plain link that shows its URL.

const LINK_PROTOCOLS = new Set(["http:", "https:", "mailto:"]);

function parse(value: string): URL | null {
  try {
    return new URL(value.trim());
  } catch {
    return null;
  }
}

/** The link target when it is an absolute `http`, `https`, or `mailto` URL, else `null`. */
export function safeLink(value: string): string | null {
  const url = parse(value);
  return url && LINK_PROTOCOLS.has(url.protocol) ? url.href : null;
}

/** Whether an image may render inline: a `blob:` URL or a `data:image/` URL. */
export function inlineImage(value: string): boolean {
  const text = value.trim().toLowerCase();
  return text.startsWith("blob:") || text.startsWith("data:image/");
}

/**
 * `react-markdown`'s URL check: an image source stays when it renders inline or can be shown as a
 * link; any other URL stays only when it is a safe link. Everything else becomes empty.
 */
export function transformUrl(value: string, key: string): string {
  if (key === "src" && inlineImage(value)) return value.trim();
  return safeLink(value) ?? "";
}
