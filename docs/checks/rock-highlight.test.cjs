const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const test = require("node:test");
const { rockFences, htmlSource, highlightSources, preprocess } = require("./rock-highlight.cjs");
const { resourceSource, verifyResourceHighlight } = require("./highlight-regression.cjs");

const script = path.join(__dirname, "rock-highlight.cjs");
function book(content) {
    return { sections: [{ Chapter: { name: "Test", content, sub_items: [] } }] };
}

test("mdbook protocol supports only HTML and works outside the repository", () => {
    for (const renderer of ["html", "markdown"]) {
        const result = spawnSync(process.execPath, [script, "supports", renderer], { cwd: os.tmpdir() });
        assert.equal(result.status, renderer === "html" ? 0 : 1);
        assert.equal(result.stdout.toString(), "");
    }
    const result = spawnSync(process.execPath, [script], {
        cwd: os.tmpdir(), encoding: "utf8",
        input: JSON.stringify([{ renderer: "html" }, book("```rock\n" + resourceSource + "```\n")]),
    });
    assert.equal(result.status, 0, result.stderr);
    const content = JSON.parse(result.stdout).sections[0].Chapter.content;
    verifyResourceHighlight(content.match(/<code[^>]*>([\s\S]*?)<\/code>/)[1]);
});

test("fences respect delimiter lengths, non-Rock blocks, and unclosed fences", () => {
    const markdown = "````markdown\n```rock\nnot Rock\n```\n````\n" +
        "`````rock\n// ```\nmain = ->\n    0\n`````\n" +
        "~~~rock\n// tilde fence\n~~~\n```rust\nlet x = 1;\n```\n";
    assert.deepEqual(rockFences(markdown).map((block) => block.source), ["// ```\nmain = ->\n    0\n", "// tilde fence\n"]);
    assert.equal(rockFences("```rock\nmain = -> 0")[0].source, "main = -> 0");
    assert.equal(rockFences("```rk\nmain = -> 0\n```\n")[0].source, "main = -> 0\n");
    assert.deepEqual(rockFences("    ```rock\n    not a fence\n    ```\n"), []);
    const unchanged = book("```rust\nlet x = 1;\n```\n");
    assert.deepEqual(preprocess({ renderer: "html" }, structuredClone(unchanged)), unchanged);
    assert.deepEqual(preprocess({ renderer: "markdown" }, structuredClone(unchanged)), unchanged);
    const modernBook = { items: unchanged.sections };
    assert.deepEqual(preprocess({ renderer: "html" }, structuredClone(modernBook)), modernBook);
});

test("batch highlighting preserves blank lines, multiline strings, entities, tabs, Unicode, and EOF", () => {
    const sources = [resourceSource, "", "\n\n", "// & < > \" ' café\n\nmain = ->\n\t0  \n",
        'main = ->\n    "first\nsecond <&>"\n    0', "// Windows\r\nmain = ->\r\n    0\r\n"];
    const highlighted = highlightSources(sources);
    assert.deepEqual(highlighted.map(htmlSource), sources);
    verifyResourceHighlight(highlighted[0]);
    assert.ok(!highlighted.some((html) => /<table|class=line|<style/.test(html)));
});

test("nested chapters use nohighlight and leave other Markdown untouched", () => {
    const input = book("Before\n\n```rust\nlet x = 1;\n```\n\nAfter\n");
    input.sections[0].Chapter.sub_items = book("````rock\n" + resourceSource + "````\n").sections;
    const result = preprocess({ renderer: "html" }, structuredClone(input));
    assert.equal(result.sections[0].Chapter.content, input.sections[0].Chapter.content);
    assert.match(result.sections[0].Chapter.sub_items[0].Chapter.content,
        /<pre><code class="language-rock nohighlight" data-rock-highlight="tree-sitter">/);
});

test("arguments retain their role in expressions, calls, closures, and reassignment", () => {
    const source = `compute = value, callback ->
    local = value + 1
    callback value
    value.println!
    value = value + local
    inner = extra -> value + extra
    value

other = ->
    value = 3
    value + 1
`;
    const [html] = highlightSources([source]);
    assert.equal(htmlSource(html), source);
    const spans = [...html.matchAll(/<span class='([^']+)'>([^<]*)<\/span>/g)];
    const classes = (name) => spans.filter((span) => span[2] === name).map((span) => span[1]);
    assert.deepEqual(classes("value"), [...Array(8).fill("variable parameter"), "variable", "variable"]);
    assert.deepEqual(classes("callback"), ["variable parameter", "variable parameter"]);
    assert.deepEqual(classes("extra"), ["variable parameter", "variable parameter"]);
    assert.deepEqual(classes("local"), ["variable", "variable"]);
    assert.deepEqual(classes("println"), ["function"]);
});

test("member names and pattern binders do not inherit an unrelated argument color", () => {
    const source = `inspect = value, object ->
    object.value
    object.value!
    match object
        Some value => value
        _ => value
    value
`;
    const [html] = highlightSources([source]);
    const values = [...html.matchAll(/<span class='([^']+)'>value<\/span>/g)].map((span) => span[1]);
    assert.deepEqual(values, ["variable parameter", "property", "function", "variable", "variable",
        "variable parameter", "variable parameter"]);
});

test("do keywords, bind arrows, and nested bindings retain their lexical roles", () => {
    const source = `sequence = value, source ->
    result = do
        value <- source 0
        nested = do
            inner <- source value
            pure inner
        _ <- nested
        pure value
    value
`;
    const [html] = highlightSources([source]);
    assert.equal(htmlSource(html), source);
    const spans = [...html.matchAll(/<span class='([^']+)'>([^<]*)<\/span>/g)];
    const classes = (name) => spans.filter((span) => span[2] === name).map((span) => span[1]);
    assert.deepEqual(classes("do"), ["keyword", "keyword"]);
    assert.deepEqual(classes("&lt;-"), Array(3).fill("operator arrow"));
    assert.deepEqual(classes("value"), ["variable parameter", "variable",
        "variable", "variable", "variable parameter"]);
    assert.deepEqual(classes("inner"), ["variable", "variable"]);
    assert.deepEqual(classes("source"), Array(3).fill("variable parameter"));
    assert.deepEqual(classes("pure"), ["function", "function"]);
});

test("spaced operators keep operands out of the function category", () => {
    const source = `main = ->
    left = 2
    right = 3
    left + right
    left * right
    left & right
    left <+> right
    object.field + right
    consume -left
    consume &left
    consume &mut left
    consume left
    0
`;
    const [html] = highlightSources([source]);
    const functions = [...html.matchAll(/<span class='function'>([^<]*)<\/span>/g)].map((span) => span[1]);
    assert.deepEqual(functions, ["main", "consume", "consume", "consume", "consume"]);
    assert.match(html, /class='property'>field<\/span>/);
});

test("CLI failures are helpful, leave stdout empty, and clean temporary files", () => {
    const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "rock-book-test-"));
    try {
        const result = spawnSync(process.execPath, [script], {
            env: { ...process.env, PATH: temporary, TMPDIR: temporary }, encoding: "utf8",
            input: JSON.stringify([{ renderer: "html" }, book("```rock\nmain = -> 0\n```\n")]),
        });
        assert.equal(result.status, 1);
        assert.equal(result.stdout, "");
        assert.match(result.stderr, /Install tree-sitter-cli 0\.26\.9/);
        assert.deepEqual(fs.readdirSync(temporary), []);
    } finally {
        fs.rmSync(temporary, { recursive: true, force: true });
    }
});

test("enum variants have one role in declarations, construction, and patterns", () => {
    const source = `make_short = -> Data 7

enum Signal
    Idle
    Data I64
    Detailed
        value: I64

struct Record
    < value: I64

make = -> Signal::Data 7

main = ->
    first = Signal::Idle
    second = Signal::Data 1
    short = Data 2
    bare = Idle
    detailed = Signal::Detailed value: 3
    record = Record value: 4
    match second
        Signal::Data value => value
        Signal::Idle => 0
    match short
        Data value => value
        Idle => 0
    match detailed
        Signal::Detailed value: value => value
    0
`;
    const [html] = highlightSources([source]);
    assert.equal(htmlSource(html), source);
    const spans = [...html.matchAll(/<span class='([^']+)'>([^<]*)<\/span>/g)];
    for (const name of ["Idle", "Data", "Detailed"]) {
        const variants = spans.filter((span) => span[2] === name);
        assert.ok(variants.length >= 3, `Exercise declaration, construction, and matching for ${name}`);
        assert.ok(variants.every((span) => span[1] === "constant variant"), `${name} must always be a variant`);
    }
    for (const name of ["Signal", "Record", "I64"]) {
        assert.ok(spans.filter((span) => span[2] === name).every((span) => span[1] === "type"));
    }
});

test("imported and type-qualified variants preserve their owner and trait colors", () => {
    const source = `> library::Signal::Data

annotation: Container::Data -> I64

main = ->
    short = Data 1
    qualified = library::Signal::Data 2
    generic = (Option I64)::Some 3
    section = (Option _)::Some 4
    trait_call = (Result _)::Applicative::pure 5
    0
`;
    const [html, isolated] = highlightSources([source, "main = ->\n    Data\n"]);
    assert.equal(htmlSource(html), source);
    const data = [...html.matchAll(/<span class='([^']+)'>Data<\/span>/g)].map((span) => span[1]);
    assert.deepEqual(data, ["constant variant", "type", "constant variant", "constant variant"]);
    assert.equal((html.match(/class='constant variant'>Some<\/span>/g) || []).length, 2);
    assert.match(html, /class='type'>Applicative<\/span>/);
    assert.match(html, /class='function'>pure<\/span>/);
    assert.match(isolated, /class='type'>Data<\/span>/);
});

test("colorful palettes exceed AA text contrast in dark and light themes", () => {
    const css = fs.readFileSync(path.join(__dirname, "../theme/rock.css"), "utf8");
    function luminance(hex) {
        const rgb = hex.match(/[\da-f]{2}/g).map((channel) => {
            const value = parseInt(channel, 16) / 255;
            return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
        });
        return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
    }
    const palettes = [...css.matchAll(/\{([^{}]*--rock-code-bg:[^{}]*)\}/g)];
    assert.equal(palettes.length, 2);
    for (const [, palette] of palettes) {
        const colors = Object.fromEntries([...palette.matchAll(/--rock-code-([\w-]+):\s*#([\da-f]{6})/g)]
            .map((match) => [match[1], luminance(match[2])]));
        for (const [name, value] of Object.entries(colors)) {
            if (name === "bg" || name === "border") continue;
            const contrast = (Math.max(value, colors.bg) + 0.05) / (Math.min(value, colors.bg) + 0.05);
            const minimum = name === "comment" ? 4.5 : 5.5;
            assert.ok(contrast >= minimum, `${name} contrast ${contrast.toFixed(2)} must be at least ${minimum}:1`);
        }
    }
});
