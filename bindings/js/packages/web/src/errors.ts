/** The error codes of the API; see `VertepError`. */
export type ErrorCode =
  | "ChartInvalid"
  | "StorageUnavailable"
  | "StoreLocked"
  | "RequestTimeout"
  | "WorkerTerminated"
  | "TransportRefused"
  | "NotFound"
  | "InvalidArgument"
  | "Activation"
  | "Delivery"
  | "Store";

/** One problem of an invalid chart, with its location in the source. */
export interface Violation {
  readonly message: string;
  /** 1-based line, 0 when unknown. */
  readonly line: number;
  /** 1-based column, 0 when unknown. */
  readonly column: number;
  /** The offending node (a state id, a YAML path), when known. */
  readonly path?: string;
}

/** Every error the API rejects with. */
export class VertepError extends Error {
  readonly code: ErrorCode;
  readonly violations: readonly Violation[];

  constructor(code: ErrorCode, message: string, violations: readonly Violation[] = []) {
    super(message);
    this.name = "VertepError";
    this.code = code;
    this.violations = violations;
  }
}

/** The JSON form of an error on the worker protocol. */
export interface WireError {
  code: ErrorCode;
  message: string;
  violations?: Violation[];
}

/** An error as it crosses the worker boundary. */
export function toWire(e: unknown): WireError {
  if (e instanceof VertepError) {
    return { code: e.code, message: e.message, violations: [...e.violations] };
  }
  const text = e instanceof Error ? e.message : String(e);
  if (text.startsWith("vertep:")) {
    try {
      const j = JSON.parse(text.slice("vertep:".length));
      return { code: j.code, message: j.message, violations: j.violations };
    } catch {
      // fall through
    }
  }
  return { code: "InvalidArgument", message: text };
}

/** The error a wire error describes. */
export function fromWire(w: WireError): VertepError {
  return new VertepError(w.code, w.message, w.violations ?? []);
}
