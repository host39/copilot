import { Placeholder } from "@web/components/Placeholder/Placeholder";
import { ThemePicker } from "@web/components/Settings/ThemePicker";

export default function AccountPage() {
  return (
    <Placeholder title="Account">
      <section aria-labelledby="appearance" className="card-panel">
        <h2 id="appearance" className="text-base font-semibold">
          Appearance
        </h2>
        <div className="mt-4">
          <ThemePicker />
        </div>
      </section>
    </Placeholder>
  );
}
