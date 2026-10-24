// Stops a Unix-only script on Windows with the supported alternative instead of a late failure
// in sh, python3 or a crate compiled only for Unix. Usage: node scripts/unix-only.mjs <alternative>
if (process.platform === "win32") {
  console.error(`This script needs a Unix host (macOS, Linux or WSL). On Windows, run npm run ${process.argv[2]}.`);
  process.exitCode = 1;
}
