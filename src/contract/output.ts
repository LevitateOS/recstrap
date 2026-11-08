
/** One line of `recstrap --json`: one JSON event per line; null when the line
 * isn't one. */
export function readStrapEvent(line: string): StrapEvent | null {
	try {
		const value = JSON.parse(line) as { event?: unknown };
		return typeof value?.event === "string" ? (value as StrapEvent) : null;
	} catch {
		return null;
	}
}

/** A failed recstrap or recfstab run's error: the last JSON object on stderr (messages
 * from mount or cp can come before it); null when there is none. */
export function readErrorPayload(stderr: string): ErrorPayload | null {
	const trimmed = stderr.trim();
	const start = trimmed.lastIndexOf("\n{");
	const text = start >= 0 ? trimmed.slice(start + 1) : trimmed.startsWith("{") ? trimmed : "";
	try {
		const value = JSON.parse(text) as Partial<ErrorPayload>;
		return value?.schema_version === ERROR_SCHEMA_VERSION && typeof value.code === "string"
			? (value as ErrorPayload)
			: null;
	} catch {
		return null;
	}
}
