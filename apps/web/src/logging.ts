// The only module in the web app allowed to write to the console. Messages are dotted keys
// (`web.api.request.failed`); put ids and status codes in `fields`.

type LogLevel = "debug" | "info" | "warn" | "error";

export function log(level: LogLevel, message: string, fields?: Record<string, unknown>): void {
  if (import.meta.env.PROD && (level === "debug" || level === "info")) {
    return;
  }

  const line = JSON.stringify({
    timestamp: new Date().toISOString(),
    level,
    service: "web",
    message,
    ...fields,
  });

  if (level === "error") {
    console.error(line);
  } else if (level === "warn") {
    console.warn(line);
  } else {
    console.debug(line);
  }
}
