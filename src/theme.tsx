import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";

export type Theme = "light" | "dark";
/** User preference: an explicit theme, or follow the OS. */
export type ThemePref = Theme | "system";

const STORAGE_KEY = "solayge.theme";

export function getStoredPref(): ThemePref {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    return v === "light" || v === "dark" || v === "system" ? v : "system";
  } catch {
    return "system";
  }
}

/** OS preference; dark when it can't be determined. */
export function systemTheme(): Theme {
  try {
    return window.matchMedia?.("(prefers-color-scheme: light)").matches
      ? "light"
      : "dark";
  } catch {
    return "dark";
  }
}

export function resolveTheme(pref: ThemePref): Theme {
  return pref === "system" ? systemTheme() : pref;
}

/** Mirror the resolved theme onto <html> so CSS tokens switch. */
export function applyTheme(theme: Theme) {
  const root = document.documentElement;
  root.classList.toggle("theme-dark", theme === "dark");
  root.classList.toggle("theme-light", theme === "light");
  root.dataset.theme = theme;
  root.style.colorScheme = theme;
}

function persist(pref: ThemePref) {
  try {
    localStorage.setItem(STORAGE_KEY, pref);
  } catch {
    /* storage is optional */
  }
}

interface ThemeContextValue {
  /** What the user chose. */
  pref: ThemePref;
  /** The theme actually applied right now. */
  theme: Theme;
  setPref: (pref: ThemePref) => void;
  /** Flip to the opposite of the current effective theme. */
  toggle: () => void;
}

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [pref, setPrefState] = useState<ThemePref>(() => getStoredPref());
  const [system, setSystem] = useState<Theme>(() => systemTheme());

  const theme: Theme = pref === "system" ? system : pref;

  useEffect(() => {
    applyTheme(theme);
  }, [theme]);

  // Track OS changes while "system" is selected.
  useEffect(() => {
    if (pref !== "system") return;
    const mq = window.matchMedia?.("(prefers-color-scheme: light)");
    if (!mq) return;
    const onChange = () => setSystem(mq.matches ? "light" : "dark");
    onChange();
    mq.addEventListener?.("change", onChange);
    return () => mq.removeEventListener?.("change", onChange);
  }, [pref]);

  const setPref = useCallback((next: ThemePref) => {
    persist(next);
    setPrefState(next);
  }, []);

  const toggle = useCallback(() => {
    setPrefState((current) => {
      const effective = current === "system" ? systemTheme() : current;
      const next: Theme = effective === "dark" ? "light" : "dark";
      persist(next);
      return next;
    });
  }, []);

  return (
    <ThemeContext.Provider value={{ pref, theme, setPref, toggle }}>
      {children}
    </ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) throw new Error("useTheme must be used inside <ThemeProvider>");
  return ctx;
}
