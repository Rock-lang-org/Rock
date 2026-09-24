const fs = require("fs");
const path = require("path");
const { rockFences, htmlSource } = require("./rock-highlight.cjs");
const { resourceSource, verifyResourceHighlight } = require("./highlight-regression.cjs");

const docsRoot = path.resolve(__dirname, "..");
const sourceRoot = path.join(docsRoot, "src");
const outputRoot = path.join(docsRoot, "book");

function walk(directory, predicate) {
    const results = [];
    for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
        const entryPath = path.join(directory, entry.name);
        if (entry.isDirectory()) {
            results.push(...walk(entryPath, predicate));
        } else if (predicate(entryPath)) {
            results.push(entryPath);
        }
    }
    return results;
}

function fail(message) {
    console.error(message);
    process.exit(1);
}

if (!fs.existsSync(outputRoot)) {
    fail("The rendered book is missing; run `mdbook build docs` first.");
}

const summaryPath = path.join(sourceRoot, "SUMMARY.md");
const summary = fs.readFileSync(summaryPath, "utf8");
const chapterLinks = [...summary.matchAll(/\]\(([^)]+\.md)\)/g)].map((match) => match[1]);
const missingChapters = chapterLinks.filter((chapter) => !fs.existsSync(path.join(sourceRoot, chapter)));
if (missingChapters.length > 0) {
    fail(`Missing SUMMARY targets:\n${missingChapters.join("\n")}`);
}

const markdownFiles = walk(sourceRoot, (file) => file.endsWith(".md"));
const listedChapters = new Set(chapterLinks.map((chapter) => path.resolve(sourceRoot, chapter)));
const orphanedChapters = markdownFiles.filter(
    (file) => file !== summaryPath && !listedChapters.has(path.resolve(file)),
);
if (orphanedChapters.length > 0) {
    fail(`Markdown pages missing from SUMMARY.md:\n${orphanedChapters.join("\n")}`);
}

let rockFenceCount = 0;
let resourceVerified = false;
let doVerified = false;
for (const file of markdownFiles) {
    const source = fs.readFileSync(file, "utf8");
    // Hyphenated names such as Rock-lang-org are not the standalone keyword.
    if (/(?<![\w-])lang(?![\w-])/.test(source)) {
        fail(`Compiler-internal \`lang\` syntax is not allowed in the user book: ${file}`);
    }
    if (/--debug-print|\bHIR\b|\bMIR\b|ast-full/.test(source)) {
        fail(`Compiler-internal debug views are not allowed in the user book: ${file}`);
    }

    const fences = rockFences(source);
    for (const fence of fences) {
        if (/\.\.\.|\bTODO\b|\/\/\s*(?:body|implementation|omitted)\b/i.test(fence.source)) {
            fail(`Rock fence contains an omission or placeholder in ${file}:\n${fence.source}`);
        }
    }
    rockFenceCount += fences.length;
    if (fences.length) {
        const renderedPath = path.join(outputRoot, path.relative(sourceRoot, file).replace(/\.md$/, ".html"));
        const rendered = fs.readFileSync(renderedPath, "utf8");
        const blocks = [...rendered.matchAll(/<pre><code\b([^>]*)>([\s\S]*?)<\/code><\/pre>/g)]
            .filter((match) => /class="[^"]*\blanguage-rock\b/.test(match[1]));
        if (blocks.length !== fences.length) fail(`Rock block count mismatch in ${renderedPath}`);
        for (let index = 0; index < fences.length; index++) {
            const [, attributes, html] = blocks[index];
            if (!/\bnohighlight\b/.test(attributes) || !/data-rock-highlight="tree-sitter"/.test(attributes)) {
                fail(`Missing build-time highlighting marker or nohighlight in ${renderedPath}`);
            }
            if (htmlSource(html) !== fences[index].source) fail(`Rock source changed in ${renderedPath}, block ${index + 1}`);
            if (!/<span class=['"]/.test(html)) fail(`Missing AST spans in ${renderedPath}, block ${index + 1}`);
            if (fences[index].source === resourceSource) {
                verifyResourceHighlight(html);
                resourceVerified = true;
            }
            if (path.relative(sourceRoot, file) === "functional/error-handling.md" &&
                fences[index].source.includes("sum_positive = left, right -> do")) {
                if (!/<span class=['"]keyword['"]>do<\/span>/.test(html) ||
                    (html.match(/<span class=['"]operator arrow['"]>&lt;-<\/span>/g) || []).length !== 2) {
                    fail("The do-notation example is missing its keyword or bind-arrow highlighting.");
                }
                doVerified = true;
            }
        }
    }
}
if (rockFenceCount === 0) {
    fail("The book contains no Rock code fences.");
}

const bookConfig = fs.readFileSync(path.join(docsRoot, "book.toml"), "utf8");
const siteUrlMatch = bookConfig.match(/^site-url\s*=\s*"([^"]+)"/m);
const siteUrl = siteUrlMatch ? siteUrlMatch[1] : "/";
const htmlFiles = walk(outputRoot, (file) => file.endsWith(".html"));
const brokenLinks = [];
let renderedRockBlocks = 0;

for (const file of htmlFiles) {
    const html = fs.readFileSync(file, "utf8");
    renderedRockBlocks += (html.match(/data-rock-highlight="tree-sitter"/g) || []).length;

    for (const match of html.matchAll(/href="([^"]+)"/g)) {
        const href = match[1];
        if (/^(?:https?:|mailto:|javascript:|#)/.test(href)) {
            continue;
        }

        const target = href.split(/[?#]/)[0];
        if (!target) {
            continue;
        }

        let resolved;
        if (siteUrl !== "/" && target.startsWith(siteUrl)) {
            resolved = path.join(outputRoot, target.slice(siteUrl.length) || "index.html");
        } else {
            resolved = path.resolve(path.dirname(file), target);
        }

        if (!fs.existsSync(resolved)) {
            brokenLinks.push(`${path.relative(outputRoot, file)} -> ${href}`);
        }
    }
}

if (brokenLinks.length > 0) {
    fail(`Broken rendered links:\n${brokenLinks.join("\n")}`);
}
if (renderedRockBlocks < rockFenceCount) {
    fail(`Only ${renderedRockBlocks} of ${rockFenceCount} Rock fences were rendered with language-rock.`);
}

const indexHtml = fs.readFileSync(path.join(outputRoot, "index.html"), "utf8");
// mdBook can fingerprint asset names; validate and load the rendered paths.
[
    /href="(theme\/rock(?:-[a-f0-9]+)?\.css)"/,
    /src="(highlight(?:-[a-f0-9]+)?\.js)"/,
].map((pattern) => {
    const match = indexHtml.match(pattern);
    if (!match || !fs.existsSync(path.join(outputRoot, match[1]))) {
        fail(`The rendered book is missing a required asset: ${pattern}`);
    }
    return path.join(outputRoot, match[1]);
});

if (!resourceVerified) fail("The rendered Resource/Drop AST highlighting regression was not exercised.");
if (!doVerified) fail("The rendered do-notation AST highlighting regression was not exercised.");

console.log(
    `Verified ${chapterLinks.length} chapters, ${rockFenceCount} Rock fences, ` +
        `${htmlFiles.length} HTML pages, rendered links, and Rock syntax highlighting.`,
);
