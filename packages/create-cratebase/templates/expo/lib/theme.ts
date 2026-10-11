import { useColorScheme } from "react-native";

const light = {
  background: "#f6f4ef",
  card: "#ffffff",
  text: "#16181d",
  muted: "#6b6f76",
  border: "#e2dfd8",
  accent: "#d9531e",
  link: "#b8430f",
  danger: "#c62828",
};

const dark: typeof light = {
  background: "#16181d",
  card: "#1f2229",
  text: "#f6f4ef",
  muted: "#9aa0a8",
  border: "#2f333b",
  accent: "#d9531e",
  link: "#ff8552",
  danger: "#ff6b6b",
};

export type Theme = typeof light;

/** Light or dark palette, following the device setting (`userInterfaceStyle: "automatic"` in app.json). */
export function useTheme(): Theme {
  return useColorScheme() === "dark" ? dark : light;
}
