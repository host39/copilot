import { ErrorView } from "@web/components/Error/ErrorView";
import { PageTitle } from "@web/components/Layout/PageTitle";
import type { FC } from "react";

const ErrorPage: FC = () => {
  return (
    <>
      <PageTitle page="Something Went Wrong" />
      <ErrorView />
    </>
  );
};

export default ErrorPage;
