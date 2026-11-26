import type { ButtonHTMLAttributes, FC, PropsWithChildren } from "react";
import { LuLoaderCircle } from "react-icons/lu";

type ButtonVariant = "primary" | "secondary" | "danger" | "ghost";

type ButtonProps = {
  variant?: ButtonVariant;
  /** Shows a spinner and disables the button while an action runs. */
  busy?: boolean;
} & ButtonHTMLAttributes<HTMLButtonElement>;

const VARIANT_CLASS_MAP: Record<ButtonVariant, string> = {
  primary: "bg-accent text-on-accent hover:bg-accent-hover",
  secondary: "border border-line bg-canvas text-fg hover:bg-raised",
  danger: "bg-danger text-canvas hover:opacity-90",
  ghost: "text-fg hover:bg-raised",
};

const Button: FC<PropsWithChildren<ButtonProps>> = ({
  children,
  className,
  variant = "primary",
  busy = false,
  disabled,
  type = "button",
  ...rest
}) => {
  const variantClass = VARIANT_CLASS_MAP[variant];
  const baseClassName =
    "inline-flex min-h-10 items-center justify-center gap-2 rounded-lg px-4 py-2 text-sm font-medium transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed disabled:opacity-60";

  return (
    <button
      type={type}
      disabled={disabled || busy}
      aria-busy={busy || undefined}
      className={`${baseClassName} ${variantClass} ${className ?? ""}`}
      {...rest}
    >
      {busy ? <LuLoaderCircle className="size-4 animate-spin" aria-hidden /> : null}
      {children}
    </button>
  );
};

export default Button;
