// Run a wasm32-wasip1 test binary under Node's WASI implementation and exit with its status.
// Usage: node wasi-run.mjs <file.wasm> [test args...]
import { readFile } from "node:fs/promises";
import { WASI } from "node:wasi";
import { argv, exit } from "node:process";

const [file, ...args] = argv.slice(2);
const wasi = new WASI({ version: "preview1", args: [file, ...args], env: {}, returnOnExit: true });
const module = await WebAssembly.compile(await readFile(file));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
exit(wasi.start(instance));
