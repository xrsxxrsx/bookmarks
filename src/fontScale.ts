/**
 * Interface font size, adjustable because the right size depends on the screen, the
 * eyesight and how long the session is.
 *
 * The whole interface is sized in `rem`, so one root font size scales everything together
 * — chrome, table and form — instead of needing a variable per component. The choice is
 * persisted so it survives a restart.
 */

export type FontScale = 'small' | 'normal' | 'large' | 'huge';

export const FONT_SCALES: { value: FontScale; label: string; rootPx: number }[] = [
  { value: 'small', label: '小', rootPx: 13 },
  { value: 'normal', label: '标准', rootPx: 14 },
  { value: 'large', label: '大', rootPx: 16 },
  { value: 'huge', label: '特大', rootPx: 18 },
];

const STORAGE_KEY = 'bookmarks.fontScale';

export function loadFontScale(): FontScale {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored !== null && FONT_SCALES.some((s) => s.value === stored)) {
      return stored as FontScale;
    }
  } catch {
    // Private mode or a blocked store: fall back to the default rather than failing.
  }
  return 'normal';
}

export function applyFontScale(scale: FontScale): void {
  const entry = FONT_SCALES.find((s) => s.value === scale) ?? FONT_SCALES[1];
  if (entry === undefined) return;
  document.documentElement.style.fontSize = `${entry.rootPx}px`;
  try {
    localStorage.setItem(STORAGE_KEY, scale);
  } catch {
    // Not being able to remember the choice is not worth interrupting the user for.
  }
}
