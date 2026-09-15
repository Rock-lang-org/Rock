const { execFileSync } = require("node:child_process");
const fs = require("node:fs");
const path = require("node:path");

const grammar = path.resolve(__dirname, "../.deps/tree-sitter-rock");
const revision = fs.readFileSync(path.join(__dirname, "grammar-revision.txt"), "utf8").trim();
if (!/^[a-f0-9]{40}$/.test(revision)) throw new Error("Invalid grammar revision");
fs.mkdirSync(grammar, { recursive: true });
const git = (...args) => execFileSync("git", ["-C", grammar, ...args], { encoding: "utf8" }).trim();
if (!fs.existsSync(path.join(grammar, ".git"))) git("init");
if (git("status", "--porcelain")) throw new Error(`Grammar checkout has local changes: ${grammar}`);
let current;
if (fs.existsSync(path.join(grammar, ".git/HEAD"))) {
    try { current = git("rev-parse", "--verify", "HEAD"); } catch { /* Initial checkout. */ }
}
if (current !== revision) {
    git("fetch", "--depth=1", "https://github.com/rock-lang-org/tree-sitter-rock.git", revision);
    git("checkout", "--detach", revision);
}
if (git("rev-parse", "HEAD") !== revision) throw new Error("Grammar revision verification failed");
execFileSync("tree-sitter", ["generate"], { cwd: grammar, stdio: "inherit" });
