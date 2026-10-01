// The press ripple of action and icon buttons (`.btn`, `.icon-btn`), after Material UI: a circle
// growing from the pointer (from the center for icon buttons) and fading out. One listener for
// the whole app; nothing when the system asks for reduced motion. Styles in app.css.

const RIPPLED = ".btn, .icon-btn";

export function installRipple() {
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  window.addEventListener("pointerdown", (e) => {
    if (reduced.matches || e.button !== 0) return;
    const button = (e.target as Element | null)?.closest<HTMLButtonElement>(RIPPLED);
    if (!button || button.disabled) return;
    const box = button.getBoundingClientRect();
    const centered = button.classList.contains("icon-btn");
    const x = centered ? box.width / 2 : e.clientX - box.left;
    const y = centered ? box.height / 2 : e.clientY - box.top;
    // Large enough to cover the button from the press point.
    const radius = Math.hypot(Math.max(x, box.width - x), Math.max(y, box.height - y));
    const ripple = document.createElement("span");
    ripple.className = "ripple";
    ripple.style.left = `${x - radius}px`;
    ripple.style.top = `${y - radius}px`;
    ripple.style.width = ripple.style.height = `${radius * 2}px`;
    button.append(ripple);
    // Fade out once the press ends (the circle keeps growing meanwhile), then go.
    const release = () => {
      window.removeEventListener("pointerup", release);
      window.removeEventListener("pointercancel", release);
      ripple.classList.add("released");
    };
    window.addEventListener("pointerup", release);
    window.addEventListener("pointercancel", release);
    ripple.addEventListener("transitionend", () => ripple.remove());
  });
}
