/** The part of a click event `followExternal` uses. */
export interface LinkClick {
  defaultPrevented: boolean;
  preventDefault(): void;
}

/**
 * Opens `href` outside the webview instead of navigating it. A click a handler already prevented is
 * left alone. Returns whether it opened the link.
 */
export function followExternal(
  e: LinkClick,
  href: string,
  open: (url: string) => Promise<void> | void,
): boolean {
  if (e.defaultPrevented) return false;
  e.preventDefault();
  void open(href);
  return true;
}
