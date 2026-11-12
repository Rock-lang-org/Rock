const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const { rockFences } = require("./rock-highlight.cjs");

const root = path.resolve(__dirname, "../..");
const compiler = path.resolve(process.env.ROCKC || path.join(root, "target/release/rockc"));
const httpRoot = path.resolve(process.env.ROCK_HTTP_ROOT || path.join(root, "../rock_http"));
const output = fs.mkdtempSync(path.join(os.tmpdir(), "rock-doc-compile-"));

function walk(directory) {
    return fs.readdirSync(directory, { withFileTypes: true }).flatMap(entry => {
        const file = path.join(directory, entry.name);
        return entry.isDirectory() ? walk(file) : [file];
    });
}

function write(file, source) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, source);
}

function compile(entry, directory, dependencies, extra = []) {
    fs.mkdirSync(directory, { recursive: true });
    const run = spawnSync(compiler, [
        "--entry-file", entry, "--output-dir", directory, "--no-std", "--no-link",
        ...dependencies.flatMap(([name, artifact]) => ["--extern-artifact", `${name}=${artifact}`]),
        ...extra,
    ], { encoding: "utf8", timeout: 120000, maxBuffer: 16 * 1024 * 1024 });
    const log = `${run.stdout || ""}${run.stderr || ""}${run.error || ""}`;
    fs.writeFileSync(path.join(directory, "compile.log"), log);
    return { status: run.status, log };
}

function artifact(name, entry, dependencies) {
    const directory = path.join(output, "dependencies", name);
    const destination = path.join(directory, `${name}.rkca`);
    const result = compile(entry, directory, dependencies, [
        "--crate-name", name, "--emit-artifact", destination,
        ...(name === "std" ? ["--no-prelude"] : []),
    ]);
    if (result.status !== 0) throw new Error(`Cannot build ${name}:\n${result.log}`);
    return [name, destination];
}

function main() {
    console.log(`Compiler: ${compiler}\nLogs: ${output}`);
    const std = artifact("std", path.join(root, "std/src/lib.rk"), []);
    const examples = [];
    for (const file of [path.join(root, "README.md"), path.join(root, "std/README.md"), path.join(root, "docs/LANGUAGE_SPECIFICATION.md"), ...walk(path.join(root, "docs/src"))]
        .filter(file => file.endsWith(".md"))) {
        // The root README uses GitHub's Haskell highlighting for Rock sources.
        // Preserve offsets so diagnostics still point to the original lines.
        const markdown = fs.readFileSync(file, "utf8").replace(/^```haskell$/gm, "```rock   ");
        for (const fence of rockFences(markdown)) {
            const before = markdown.slice(0, fence.start);
            const line = before.split("\n").length;
            const sectionStart = before.lastIndexOf("\n## ") + 1;
            const sectionEnd = markdown.indexOf("\n## ", fence.end);
            const section = markdown.slice(sectionStart, sectionEnd < 0 ? undefined : sectionEnd);
            const namedFiles = [];
            if (["modules.md", "packages.md", "README.md"].includes(path.basename(file))) {
                for (const listing of rockFences(section)) {
                    const name = section.slice(0, listing.start).match(/### `([^`]+\.rk)`\s*$/)?.[1];
                    if (name) namedFiles.push([name, listing.source]);
                }
            }
            examples.push({
                location: `${path.relative(root, file)}:${line}`,
                source: fence.source,
                expected: before.match(/<!-- compile-fail: (.+?) -->\s*$/)?.[1],
                namedFiles,
                macroLibrarySource: path.basename(file) === "macros.md" && fence.source.includes("> mylib::")
                    ? `mod text\n` : undefined,
                macroModuleSource: path.basename(file) === "macros.md" && fence.source.includes("> mylib::")
                    ? rockFences(section).find(item => item.source.startsWith("< macro "))?.source : undefined,
                noStd: path.basename(file) === "packages.md" && /no_std = true/.test(section),
            });
        }
    }
    for (const file of walk(path.join(root, "docs/examples")).filter(file => file.endsWith(".rk"))) {
        examples.push({ location: path.relative(root, file), source: fs.readFileSync(file, "utf8"), namedFiles: [] });
    }

    let http;
    let geometry;
    let tools;
    let mylib;
    let passed = 0;
    let rejected = 0;
    const failures = [];
    const selected = examples.filter(example => !process.env.ROCK_DOC_FILTER || example.location.split(":")[0] === process.env.ROCK_DOC_FILTER);
    if (!selected.length) throw new Error("No examples matched ROCK_DOC_FILTER");
    for (const [index, example] of selected.entries()) {
        const directory = path.join(output, String(index + 1));
        const dependencies = example.noStd ? [] : [std];
        try {
            if (example.source.includes("> rock_http::")) {
                http ||= artifact("rock_http", path.join(httpRoot, "src/lib.rk"), [std]);
                dependencies.push(http);
            }
            for (const [name, source] of example.namedFiles) write(path.join(directory, name), source);
            if (example.source.includes("> geometry::") && example.namedFiles.some(([name]) => name === "geometry/lib.rk")) {
                geometry ||= artifact("geometry", path.join(directory, "geometry/lib.rk"), [std]);
                dependencies.push(geometry);
            }
            if (example.source.includes("> tools::") && example.namedFiles.some(([name]) => name === "tools/src/lib.rk")) {
                tools ||= artifact("tools", path.join(directory, "tools/src/lib.rk"), [std]);
                dependencies.push(tools);
            }
            if (example.macroLibrarySource) {
                if (!example.macroModuleSource) throw new Error("Missing companion macro library");
                write(path.join(directory, "mylib/lib.rk"), example.macroLibrarySource);
                write(path.join(directory, "mylib/text.rk"), example.macroModuleSource);
                mylib ||= artifact("mylib", path.join(directory, "mylib/lib.rk"), [std]);
                dependencies.push(mylib);
            }
            const name = example.namedFiles.find(([, source]) => source === example.source)?.[0] || "main.rk";
            write(path.join(directory, name), example.source);
            // README module helpers rely on inference from their companion main.
            const entryName = example.location.startsWith("README.md:") && example.namedFiles.some(([name]) => name === "main.rk")
                ? "main.rk" : name;
            const entry = path.join(directory, entryName);
            const result = compile(entry, path.join(directory, "build"), dependencies);
            if (example.expected) {
                if (result.status !== 1 || !result.log.includes(example.expected)) {
                    throw new Error(`Expected diagnostic: ${example.expected}\n${result.log}`);
                }
                rejected++;
            } else {
                if (result.status !== 0) throw new Error(result.log);
                passed++;
            }
            console.log(`${example.expected ? "EXPECTED FAILURE" : "PASS"} ${example.location}`);
        } catch (error) {
            failures.push({ location: example.location, error: error.message });
            console.error(`FAIL ${example.location}\n${error.message}`);
        }
    }
    fs.writeFileSync(path.join(output, "failures.json"), JSON.stringify(failures, null, 2));
    console.log(`${passed} compiled; ${rejected} expected failures; ${failures.length} unexpected failures.`);
    process.exitCode = failures.length ? 1 : 0;
}

try {
    main();
} catch (error) {
    console.error(error.message);
    process.exitCode = 1;
}
