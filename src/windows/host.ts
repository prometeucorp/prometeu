/** Linux execution context prepared by the native Windows host; mirrors `prometeu_bridge::Target`. */
export type Target = { distribution: string; executable: string; root: string; workdir: string; codex: string };

/** Native Windows host commands outside the addressed application IPC, which is unchanged. */
export type Commands = {
  application_reconnect: { args: undefined; result: boolean };
  application_paths: { args: { paths: string[]; direction: "linux" | "windows" }; result: string[] };
  application_open: { args: { previous: Target | null }; result: Target };
};
