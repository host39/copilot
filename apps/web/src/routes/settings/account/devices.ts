// The Devices list of Settings, Account: rustauth's sessions, named after their browser and system.

import type { SessionResponse } from "@copilot/contracts";

/** Checked in order: Edge and Opera also name Chrome, and Chrome also names Safari. */
const BROWSERS: [RegExp, string][] = [
  [/\bEdg(e|A|iOS)?\//, "Edge"],
  [/\bOPR\/|\bOpera\b/, "Opera"],
  [/\bSamsungBrowser\//, "Samsung Internet"],
  [/\bFirefox\/|\bFxiOS\//, "Firefox"],
  [/\bChrom(e|ium)\/|\bCriOS\//, "Chrome"],
  [/\bVersion\/[\d.]+.*\bSafari\//, "Safari"],
];

/** Checked in order: Android also names Linux, and iPhones name Mac OS X. */
const SYSTEMS: [RegExp, string][] = [
  [/\bAndroid\b/, "Android"],
  [/\biPhone\b|\biPad\b|\biPod\b/, "iOS"],
  [/\bWindows\b/, "Windows"],
  [/\bCrOS\b/, "ChromeOS"],
  [/\bMac OS X\b|\bMacintosh\b/, "macOS"],
  [/\bLinux\b|\bX11\b/, "Linux"],
];

const RAW_MAX_CHARS = 60;

function match(userAgent: string, table: [RegExp, string][]): string | undefined {
  return table.find(([pattern]) => pattern.test(userAgent))?.[1];
}

/**
 * A readable name for a session's user agent ("Firefox on Linux"). An agent it does not know
 * shows as itself, shortened; an empty one as "Unknown device".
 */
export function deviceName(userAgent: string): string {
  const agent = userAgent.trim();
  if (agent === "") return "Unknown device";
  const browser = match(agent, BROWSERS);
  const system = match(agent, SYSTEMS);
  if (browser && system) return `${browser} on ${system}`;
  if (browser || system) return (browser ?? system) as string;
  return agent.length > RAW_MAX_CHARS ? `${agent.slice(0, RAW_MAX_CHARS - 1)}…` : agent;
}

/** This device first, then the others, most recently used first. */
export function sortSessions(sessions: readonly SessionResponse[]): SessionResponse[] {
  return [...sessions].sort((a, b) => {
    if (a.current !== b.current) return a.current ? -1 : 1;
    return Date.parse(b.lastUsedAt) - Date.parse(a.lastUsedAt);
  });
}
