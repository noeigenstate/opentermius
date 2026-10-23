// Stroke icons (24px grid) shared across the app; colour follows `currentColor`.
const paths = {
  hosts: "M4 5h16v6H4zM4 13h16v6H4zM8 8h.01M8 16h.01",
  keychain: "M15 7a4 4 0 1 1-3.9 4.9L4 19v-3h3v-3h3l1.1-1.1A4 4 0 0 1 15 7zM16 9h.01",
  snippets: "M8 4c-2 0-2 2-2 4s-2 4-2 4 2 0 2 4 0 4 2 4M16 4c2 0 2 2 2 4s2 4 2 4-2 0-2 4 0 4-2 4",
  logs: "M12 7v5l3 2M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0z",
  bookmark: "M6 3h12v18l-6-4-6 4z",
  settings: "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1z",
  terminal: "M4 5h16v14H4zM8 10l3 2-3 2M13 15h3",
  prompt: "M5 7l5 5-5 5M12 18h7",
  logo: "M21.5 12a9.5 9.5 0 1 1-19 0 9.5 9.5 0 0 1 19 0zM8.5 9.5l3 2.5-3 2.5M13 15h3",
  more: "M6.5 12h.01M12 12h.01M17.5 12h.01",
  star: "m12 3 2.8 5.7 6.2.9-4.5 4.4 1.1 6.2-5.6-3-5.6 3 1.1-6.2L3 9.6l6.2-.9z",
  external: "M14 3h7v7M21 3l-11 11M11 3H4v17h17v-7",
  grid: "M3 3h7v7H3zM14 3h7v7h-7zM3 14h7v7H3zM14 14h7v7h-7z",
  list: "M3 5h2M9 5h12M3 12h2M9 12h12M3 19h2M9 19h12",
  plus: "M12 5v14M5 12h14",
  search: "M11 18a7 7 0 1 0 0-14 7 7 0 0 0 0 14zM21 21l-4.3-4.3",
  minimize: "M5 12h14",
  maximize: "M6 6h12v12H6z",
  close: "M6 6l12 12M18 6L6 18",
  arrowUp: "M12 19V5M6 11l6-6 6 6",
  refresh: "M20 11a8 8 0 1 0-2.3 5.7M20 5v6h-6",
  alert: "M12 4l9 16H3zM12 10v4M12 17h.01",
  folder: "M3 6h6l2 2h10v11H3z",
  file: "M6 3h8l5 5v13H6zM14 3v5h5",
  chevronDown: "M6 9l6 6 6-6",
  chevronRight: "M9 6l6 6-6 6",
  copy: "M9 9h11v11H9zM5 15V4h11",
  download: "M12 4v12M6 11l6 6 6-6M5 20h14",
  edit: "M4 20h4L19 9l-4-4L4 16zM13.5 6.5l4 4",
  trash: "M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3",
} as const;

export type IconName = keyof typeof paths;

/** `more` is three zero-length segments: its round caps need a heavier stroke to read as dots. */
const strokeWidths: Partial<Record<IconName, number>> = { more: 3.2 };

export function Icon({ name, size = 16 }: { name: IconName; size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={strokeWidths[name] ?? 1.8} strokeLinecap="round" strokeLinejoin="round" aria-hidden>
      <path d={paths[name]} />
    </svg>
  );
}
