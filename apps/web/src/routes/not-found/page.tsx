import notFoundIllustration from "@web/assets/images/404.svg";
import Button from "@web/components/Button/Button";
import { CenteredPage } from "@web/components/Layout/CenteredPage";
import { PageTitle } from "@web/components/Layout/PageTitle";
import path from "@web/routes/path";
import type { FC } from "react";
import { useNavigate } from "react-router-dom";

const NotFoundPage: FC = () => {
  const navigate = useNavigate();

  return (
    <CenteredPage>
      <PageTitle page="Page Not Found" />
      <img
        src={notFoundIllustration}
        alt="Illustration of a page not found error"
        className="mx-auto mb-6 h-56 sm:h-72 w-auto"
      />
      <p className="text-sm font-medium uppercase tracking-[0.2em] text-muted">404</p>
      <h1 className="page-heading">Page not found</h1>
      <p className="mt-3 text-sm text-muted sm:text-base">
        The page you are looking for does not exist or may have been moved.
      </p>
      <div className="mt-6 flex justify-center">
        <Button onClick={() => navigate(path.home)} variant="primary">
          Back to home
        </Button>
      </div>
    </CenteredPage>
  );
};

export default NotFoundPage;
