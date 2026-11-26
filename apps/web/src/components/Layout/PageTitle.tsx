const APP_TITLE = "Copilot";

type PageTitleProps = {
  /** Short page label in title case, for example "Page Not Found". Omit on the home page. */
  page?: string;
};

/** Sets the browser tab title to `Copilot | <page>`. React hoists `<title>` into `<head>`. */
export function PageTitle({ page }: PageTitleProps) {
  return <title>{page ? `${APP_TITLE} | ${page}` : APP_TITLE}</title>;
}
