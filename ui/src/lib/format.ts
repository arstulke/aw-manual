import type { ElapsedFormat, TimeFormat } from "./types";

export const PATH_SEPARATOR = " › ";

/** Parses durations as written by the daemon: "2s", "10s", "1500ms", "1h30m". */
export function parseDuration(s: string): number {
  let total = 0;
  const re = /(\d+(?:\.\d+)?)\s*(ms|h|m|s|d)/gi;
  let m: RegExpExecArray | null;
  let matched = false;
  while ((m = re.exec(s)) !== null) {
    matched = true;
    const n = parseFloat(m[1]);
    const unit = m[2].toLowerCase();
    total += n * ({ ms: 1, s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 } as Record<string, number>)[unit];
  }
  if (!matched) throw new Error(`invalid duration ${s}`);
  return total;
}

export function formatElapsed(totalSeconds: number, fmt: ElapsedFormat): string {
  const secs = Math.max(0, Math.floor(totalSeconds));
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  switch (fmt) {
    case "1h 23m":
      // Below one minute, "0m" says nothing: show the seconds.
      return h > 0 ? `${h}h ${m}m` : m > 0 ? `${m}m` : `${s}s`;
    case "1:23":
      return `${h}:${pad(m)}`;
    case "1h 23m 45s":
      return h > 0 ? `${h}h ${m}m ${s}s` : m > 0 ? `${m}m ${s}s` : `${s}s`;
    case "1:23:45":
      return `${h}:${pad(m)}:${pad(s)}`;
  }
}

export function formatClock(date: Date, fmt: TimeFormat): string {
  const h = date.getHours();
  const m = String(date.getMinutes()).padStart(2, "0");
  if (fmt === "24h") return `${String(h).padStart(2, "0")}:${m}`;
  const h12 = h % 12 === 0 ? 12 : h % 12;
  return `${h12}:${m} ${h < 12 ? "AM" : "PM"}`;
}

const ICONS: Record<string, string> = {
  briefcase: "💼",
  code: "💻",
  coffee: "☕",
  bug: "🐛",
  book: "📖",
  calendar: "📅",
  chart: "📊",
  chat: "💬",
  check: "✅",
  cloud: "☁️",
  design: "🎨",
  palette: "🎨",
  doc: "📄",
  document: "📄",
  email: "✉️",
  mail: "✉️",
  focus: "🎯",
  target: "🎯",
  globe: "🌐",
  web: "🌐",
  home: "🏠",
  lunch: "🍽️",
  food: "🍽️",
  meeting: "👥",
  people: "👥",
  phone: "📞",
  call: "📞",
  pause: "⏸️",
  break: "⏸️",
  rocket: "🚀",
  deploy: "🚀",
  review: "🔍",
  search: "🔍",
  server: "🖥️",
  settings: "⚙️",
  gear: "⚙️",
  star: "⭐",
  support: "🛟",
  test: "🧪",
  tools: "🛠️",
  wrench: "🔧",
  travel: "✈️",
  write: "✏️",
  pencil: "✏️",
  idea: "💡",
  money: "💰",
  invoice: "🧾",
  sport: "🏃",
  health: "❤️",
  learn: "🎓",
  music: "🎵",
};

/** An emoji for an icon name; emoji given directly are shown as they are. */
export function iconGlyph(icon: string | null | undefined): string {
  if (!icon) return "";
  const known = ICONS[icon.toLowerCase()];
  if (known) return known;
  // Not plain ASCII: treat as an emoji or symbol written directly.
  return /^[\x00-\x7F]*$/.test(icon) ? "" : icon;
}
