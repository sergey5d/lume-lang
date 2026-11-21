export interface Project { entry: string; files: Record<string, string> }
export interface Diagnostic {
  path: string;
  severity: 'error' | 'warning';
  code: string;
  message: string;
  span: { start: number; end: number; line: number; column: number; endLine: number; endColumn: number };
  label: string | null;
  notes: string[];
  helps: string[];
}
export interface LumeResult {
  protocol: 1;
  ok: boolean;
  diagnostics: Diagnostic[];
  warnings: Diagnostic[];
  output: string;
  returnValue: string | null;
  text?: string;
}
export interface RequestOptions { signal?: AbortSignal; timeoutMs?: number; onOutput?: (text: string) => void }
export interface RunOptions extends RequestOptions {
  args?: string[];
  environment?: Record<string, string>;
  entry?: string;
  /** Values can reduce, but not raise, the browser's built-in limits. */
  limits?: Partial<{ steps: number; callDepth: number; collectionItems: number; stringBytes: number; outputBytes: number; cleanupSteps: number }>;
}
export class LumeHostError extends Error { code: string }
export class LumeClient {
  constructor(options?: { wasmUrl?: string | URL; workerUrl?: string | URL; timeoutMs?: number });
  check(project: Project, options?: RequestOptions): Promise<LumeResult>;
  format(source: string, options?: RequestOptions & { path?: string }): Promise<LumeResult>;
  run(project: Project, options?: RunOptions): Promise<LumeResult>;
  dispose(): void;
}
export function byteOffsetToUtf16(source: string, bytes: number): number;
