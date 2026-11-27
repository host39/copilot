import type { UserRole } from "@copilot/contracts";
import { ROLES } from "@web/routes/settings/user/accounts";

type RolePickerProps = {
  value: UserRole;
  onChange: (role: UserRole) => void;
};

/** The account role as a radio group, each role with what it allows. */
export function RolePicker({ value, onChange }: RolePickerProps) {
  return (
    <fieldset>
      <legend className="field-label">Role</legend>
      <div className="mt-2 grid gap-2 sm:grid-cols-3">
        {ROLES.map((role) => (
          <label key={role.value} className="choice-card flex-col gap-0.5">
            <input
              type="radio"
              name="role"
              value={role.value}
              checked={value === role.value}
              onChange={() => onChange(role.value)}
              className="sr-only"
            />
            <span className="font-medium">{role.label}</span>
            <span className="text-muted">{role.hint}</span>
          </label>
        ))}
      </div>
    </fieldset>
  );
}
