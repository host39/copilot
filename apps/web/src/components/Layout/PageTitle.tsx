const APP_TITLE = "Copilot";

/** What a tab that waits for the user starts with, so it stands out among other tabs. */
export const ATTENTION_MARK = "(!)";

type PageTitleProps = {
  /**
   * Short page label in title case, for example "Page Not Found", or a name the user gave (a
   * chat's or a project's). Omit on the home page.
   */
  page?: string;
  /** The page waits for the user (a tool approval): the title starts with `ATTENTION_MARK`. */
  attention?: boolean;
};

/** The browser tab title: `Copilot | <page>`, after `ATTENTION_MARK` when the page waits. */
export function pageTitle(page?: string, attention = false): string {
  const title = page ? `${APP_TITLE} | ${page}` : APP_TITLE;
  return attention ? `${ATTENTION_MARK} ${title}` : title;
}

/** Sets the browser tab title ([`pageTitle`]). React hoists `<title>` into `<head>`. */
export function PageTitle({ page, attention = false }: PageTitleProps) {
  return <title>{pageTitle(page, attention)}</title>;
}
