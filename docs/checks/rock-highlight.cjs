const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { execFileSync } = require("node:child_process");

const repoRoot = path.resolve(__dirname, "../..");
const grammarRoot = path.join(repoRoot, "docs/.deps/tree-sitter-rock");
const queryPath = path.join(grammarRoot, "queries/highlights.scm");
const localsPath = path.join(grammarRoot, "queries/locals.scm");

// Only Markdown fence boundaries are scanned here; Rock tokens come from the AST query.
function rockFences(markdown) {
    const blocks = [];
    let fence;
    for (const match of markdown.matchAll(/[^\n]*(?:\n|$)/g)) {
        const line = match[0];
        if (!line) continue;
        const text = line.replace(/\r?\n$/, "");
        if (!fence) {
            const opening = text.match(/^( {0,3})(`{3,}|~{3,})([^\r\n]*)$/);
            if (!opening || (opening[2][0] === "`" && opening[3].includes("`"))) continue;
            fence = {
                start: match.index,
                bodyStart: match.index + line.length,
                delimiter: opening[2],
                rock: /^(?:rock|rk)(?:[\s,]|$)/.test(opening[3].trim()),
            };
        } else {
            const closing = text.match(/^ {0,3}(`{3,}|~{3,})[ \t]*$/);
            if (!closing || closing[1][0] !== fence.delimiter[0] || closing[1].length < fence.delimiter.length) continue;
            if (fence.rock) {
                blocks.push({
                    start: fence.start,
                    end: match.index + line.length,
                    source: markdown.slice(fence.bodyStart, match.index),
                });
            }
            fence = undefined;
        }
    }
    if (fence?.rock) {
        blocks.push({ start: fence.start, end: markdown.length, source: markdown.slice(fence.bodyStart) });
    }
    return blocks;
}

function htmlSource(html) {
    return html.replace(/<[^>]*>/g, "").replace(/&(#x[\da-f]+|#\d+|amp|lt|gt|quot|apos);/gi, (_, entity) => {
        if (entity[0] === "#") {
            return String.fromCodePoint(entity[1].toLowerCase() === "x"
                ? parseInt(entity.slice(2), 16) : parseInt(entity.slice(1), 10));
        }
        return { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'" }[entity];
    });
}

function highlightSources(sources) {
    if (!sources.length) return [];
    for (const file of [queryPath, localsPath]) {
        if (!fs.existsSync(file)) throw new Error(`Missing Rock AST highlight query: ${file}`);
    }
    const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rock-book-"));
    try {
        // The CLI's default theme collapses custom captures (operator.assignment -> operator).
        // Register query capture names verbatim; CSS, not this temporary theme, owns colors.
        const captures = [...fs.readFileSync(queryPath, "utf8").matchAll(/"(?:\\.|[^"\\])*"|;[^\n]*|@([\w.-]+)/g)]
            .filter((match) => match[1]).map((match) => [match[1], null]);
        const configPath = path.join(temporary, "config.json");
        fs.writeFileSync(configPath, JSON.stringify({ "parser-directories": [path.dirname(grammarRoot)], theme: Object.fromEntries(captures) }));
        const highlighted = [];
        for (let offset = 0; offset < sources.length; offset += 64) {
            const batch = sources.slice(offset, offset + 64);
            const files = batch.map((source, index) => {
                const file = path.join(temporary, `${offset + index}.rk`);
                fs.writeFileSync(file, source);
                return file;
            });
            let output;
            try {
                output = execFileSync("tree-sitter", [
                    "highlight", "--html", "--css-classes", "--scope", "source.rock",
                    "--config-path", configPath,
                    "--query-paths", queryPath, localsPath, "--", ...files,
                ], { cwd: grammarRoot, encoding: "utf8", maxBuffer: 64 * 1024 * 1024, stdio: ["ignore", "pipe", "pipe"] });
            } catch (error) {
                throw new Error(`Tree-sitter highlighting failed. Install tree-sitter-cli 0.26.9, run \`tree-sitter generate\` in ${grammarRoot}, and check ${queryPath}.\n${error.stderr || error.message}`);
            }
            // The book pins CLI 0.26 (tables); the extension pins 0.27 (pre/code).
            const tables = [...output.matchAll(/<table>([\s\S]*?)<\/table>/g)];
            const blocks = tables.length
                ? tables.map((table) => [...table[1].matchAll(/<td class=line>([\s\S]*?)<\/td>/g)].map((match) => match[1]).join(""))
                : [...output.matchAll(/<pre><code>([\s\S]*?)<\/code><\/pre>/g)].map((match) => match[1]);
            if (blocks.length !== batch.length) throw new Error("Unexpected tree-sitter HTML: expected one block per Rock snippet.");
            for (let index = 0; index < batch.length; index++) {
                let html = blocks[index];
                // Resolve only AST-captured value names, never type annotations.
                // Each fence has its own declarations, independent of source order.
                const variants = new Set([...html.matchAll(/<span class='constant variant definition'>([^<]*)<\/span>/g)]
                    .map((match) => match[1]));
                html = html.replace(/<span class='(constant variant definition|type variant_reference)'>([^<]*)<\/span>/g,
                    (_, role, name) => `<span class='${role === "constant variant definition" || variants.has(name)
                        ? "constant variant" : "type"}'>${name}</span>`);
                // Tree-sitter renders CRLF as LF; retain the author's line endings.
                const endings = batch[index].match(/\r?\n/g) || [];
                let line = 0;
                html = html.replace(/\r?\n/g, () => endings[line++] || "\n");
                // The CLI terminates the last rendered line even when the input does not.
                if (!batch[index].endsWith("\n") && htmlSource(html) === batch[index] + "\n") {
                    html = html.replace(/\n(?=(?:<\/span>)*$)/, "");
                }
                // CLI 0.27 omits the final line ending from its code block.
                const text = htmlSource(html);
                const tail = batch[index].slice(text.length);
                if (batch[index].startsWith(text) && /^(?:\r?\n)+$/.test(tail)) html += tail;
                if (htmlSource(html) !== batch[index]) throw new Error(`Tree-sitter changed source whitespace in Rock snippet ${offset + index + 1}.`);
                highlighted.push(html);
            }
        }
        return highlighted;
    } finally {
        fs.rmSync(temporary, { recursive: true, force: true });
    }
}

function preprocess(context, book) {
    if (context.renderer !== "html") return book;
    const chapters = [];
    function visit(items) {
        for (const item of items) {
            if (!item.Chapter) continue;
            const chapter = item.Chapter;
            chapters.push({ chapter, blocks: rockFences(chapter.content) });
            visit(chapter.sub_items || []);
        }
    }
    // mdBook 0.5 renamed sections to items.
    visit(book.items ?? book.sections);
    const highlighted = highlightSources(chapters.flatMap(({ blocks }) => blocks.map(({ source }) => source)));
    let index = 0;
    for (const { chapter, blocks } of chapters) {
        let content = "";
        let position = 0;
        for (const block of blocks) {
            content += chapter.content.slice(position, block.start);
            content += `<pre><code class="language-rock nohighlight" data-rock-highlight="tree-sitter">${highlighted[index++]}</code></pre>\n\n`;
            position = block.end;
        }
        chapter.content = content + chapter.content.slice(position);
    }
    return book;
}

if (require.main === module) {
    if (process.argv[2] === "supports") process.exit(process.argv[3] === "html" ? 0 : 1);
    try {
        const [context, book] = JSON.parse(fs.readFileSync(0, "utf8"));
        process.stdout.write(JSON.stringify(preprocess(context, book)));
    } catch (error) {
        console.error(`rock-highlight: ${error.message}`);
        process.exitCode = 1;
    }
}

module.exports = { rockFences, htmlSource, highlightSources, preprocess };
