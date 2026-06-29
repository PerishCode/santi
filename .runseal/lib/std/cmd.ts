//! Minimal subprocess helpers over Deno-native APIs.

export interface CaptureResult {
  code: number;
  stdout: string;
  stderr: string;
}

/** Run a command, capturing stdout/stderr. */
export async function capture(
  command: string,
  args: string[],
  options: { cwd?: string; env?: Record<string, string> } = {},
): Promise<CaptureResult> {
  const output = await new Deno.Command(command, {
    args,
    cwd: options.cwd,
    env: options.env,
    stdout: "piped",
    stderr: "piped",
  }).output();
  return {
    code: output.code,
    stdout: new TextDecoder().decode(output.stdout),
    stderr: new TextDecoder().decode(output.stderr),
  };
}

/** Run a command with inherited stdio; returns the exit code. */
export async function run(
  command: string,
  args: string[],
  options: { cwd?: string; env?: Record<string, string> } = {},
): Promise<number> {
  const output = await new Deno.Command(command, {
    args,
    cwd: options.cwd,
    env: options.env,
    stdout: "inherit",
    stderr: "inherit",
  }).output();
  return output.code;
}

export function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
