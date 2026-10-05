import { button, type ButtonVariant } from "./primitives";
import { icon, type IconName } from "./icons";

/** Decorative glyph with a mandatory accessible name. Callbacks stay in the consumer. */
export function iconButton(options: {
  label: string; glyph: IconName; run: () => void; disabled?: boolean;
  variant?: ButtonVariant; size?: number;
}) {
  const control = button("", options.run, options.variant ?? "ghost");
  control.classList.add("icon-button");
  control.innerHTML = icon(options.glyph, options.size ?? 14);
  control.title = options.label;
  control.setAttribute("aria-label", options.label);
  control.disabled = !!options.disabled;
  return control;
}
