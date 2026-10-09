#include "rock_descriptor_runtime.h"
#include <limits.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(void *) == 8 && sizeof(size_t) == 8, "Rock metadata requires a native 64-bit target");
_Static_assert(_Alignof(uint64_t) == 8 && _Alignof(void *) == 8, "Rock metadata requires 8-byte native word/pointer alignment");
_Static_assert(sizeof(RockDrop) == 8 && sizeof(RockDescriptor) == 136, "descriptor ABI mismatch");
_Static_assert(offsetof(RockDescriptor, key) == 72 && offsetof(RockDescriptor, children) == 104,
    "descriptor ABI offsets mismatch");
#define DEPTH_LIMIT 128u
#define KEY_LIMIT (16u * 1024u * 1024u)

typedef struct Record {
    struct Record *next;
    uint64_t hash, state;
    RockDescriptor descriptor;
} Record;
static pthread_mutex_t metadata_mutex = PTHREAD_MUTEX_INITIALIZER;
static Record *records;

static uint64_t read_word(const unsigned char *p) {
    uint64_t word = 0;
    for (unsigned i = 0; i < 8; ++i) word |= (uint64_t)p[i] << (8 * i);
    return word;
}
static void write_word(unsigned char *p, uint64_t word) {
    for (unsigned i = 0; i < 8; ++i) p[i] = (unsigned char)(word >> (8 * i));
}
static int equal_bytes(uint64_t na, const unsigned char *a, uint64_t nb, const unsigned char *b) {
    return na == nb && (!na || (a && b && !memcmp(a, b, (size_t)na)));
}
static uint64_t key_hash(const unsigned char *key, uint64_t len) {
#ifdef ROCK_METADATA_TEST_COLLISIONS
    (void)key; (void)len; return 0;
#else
    uint64_t hash = UINT64_C(14695981039346656037);
    for (uint64_t i = 0; i < len; ++i) hash = (hash ^ key[i]) * UINT64_C(1099511628211);
    return hash;
#endif
}
static int take_word(const unsigned char **p, uint64_t *left, uint64_t *word) {
    if (*left < 8) return 0;
    *word = read_word(*p); *p += 8; *left -= 8; return 1;
}
static int take_frame(const unsigned char **p, uint64_t *left,
    const unsigned char **bytes, uint64_t *len) {
    if (!take_word(p, left, len) || *len > *left) return 0;
    *bytes = *p; *p += *len; *left -= *len; return 1;
}
static int valid_definition(const unsigned char *p, uint64_t left) {
    const unsigned char *text; uint64_t len, count;
    for (unsigned i = 0; i < 2; ++i) if (!take_frame(&p, &left, &text, &len)) return 0;
    for (unsigned i = 0; i < 3; ++i) {
        if (!left || *p > 1) return 0;
        unsigned present = *p++; --left;
        if (present && !take_frame(&p, &left, &text, &len)) return 0;
    }
    if (left < 4) return 0;
    p += 4; left -= 4;
    if (!take_word(&p, &left, &count) || count > left / 8) return 0;
    for (uint64_t i = 0; i < count; ++i) if (!take_frame(&p, &left, &text, &len)) return 0;
    return left == 4; /* defining item ordinal, never importer crate ID */
}
static int valid_nominal_template(const unsigned char *, uint64_t);
static int valid_key(const unsigned char *p, uint64_t left, const char *magic, unsigned depth) {
    if (!p || left < 21 || left > KEY_LIMIT || depth >= DEPTH_LIMIT || memcmp(p, magic, 4)) return 0;
    unsigned char tag = p[4]; p += 5; left -= 5;
    const unsigned char *payload, *child; uint64_t payload_len, count, child_len;
    if (!take_frame(&p, &left, &payload, &payload_len) || !take_word(&p, &left, &count) || count > left / 8) return 0;
    if (!memcmp(magic, "RK01", 4)) {
        switch (tag) {
        case 'a': {
            static const char *atoms[] = {"i8","i16","i32","i64","u8","u16","u32","u64","f32","f64","bool","char","str","unit","never"};
            int found = 0;
            for (unsigned i = 0; i < sizeof(atoms) / sizeof(atoms[0]); ++i)
                if (equal_bytes(payload_len, payload, strlen(atoms[i]), (const unsigned char *)atoms[i])) found = 1;
            if (!found || count) return 0;
            break;
        }
        case 't': if (payload_len) return 0; break;
        case 'p': case 'r': case 'R': case 's': if (payload_len || count != 1) return 0; break;
        case 'A': if (payload_len != 8 || count != 1) return 0; break;
        case 'S': case 'E': if (!valid_definition(payload, payload_len)) return 0; break;
        default: return 0;
        }
    } else {
        if (tag != 't' && tag != 'n' && tag != 'f' && tag != 'a' && tag != 'g') return 0;
        if ((tag == 't' && payload_len) || (tag == 'n' && count) || (tag == 'a' && count != 1)) return 0;
        if (tag != 't') {
            /* Actual drop-plan payload includes a framed semantic identity.
             * Callable definition/signature data is compiler-validated; no
             * address or runtime impl selection participates in this key. */
            const unsigned char *q = payload, *semantic; uint64_t remaining = payload_len, n;
            if (!take_frame(&q, &remaining, &semantic, &n) || !valid_key(semantic, n, "RK01", depth + 1) || !remaining || *q > 1) return 0;
            unsigned user = *q++; --remaining;
            if (user) {
                const unsigned char *def; uint64_t def_len, params;
                if (!take_frame(&q, &remaining, &def, &def_len) || !valid_definition(def, def_len) || !take_word(&q, &remaining, &params) || params > remaining / 9) return 0;
                for (uint64_t i = 0; i < params; ++i) {
                    if (!take_frame(&q, &remaining, &semantic, &n) || !valid_key(semantic, n, "RK01", depth + 1) || !remaining || *q > 2) return 0;
                    ++q; --remaining;
                }
                for (unsigned i = 0; i < 2; ++i)
                    if (!take_frame(&q, &remaining, &semantic, &n) || !valid_key(semantic, n, "RK01", depth + 1)) return 0;
            }
            if (tag == 'g') {
                const unsigned char *template_key; uint64_t template_len;
                if (user || !take_frame(&q, &remaining, &template_key, &template_len) ||
                    !valid_nominal_template(template_key, template_len)) return 0;
            }
            if (remaining != (tag == 'a' ? 8u : 0u)) return 0;
        }
    }
    for (uint64_t i = 0; i < count; ++i)
        if (!take_frame(&p, &left, &child, &child_len) || !valid_key(child, child_len, magic, depth + 1)) return 0;
    return left == 0;
}
static RockMetadataStatus target_valid(const RockDescriptorTarget *target) {
    const uint16_t endian = 1;
    if (!target || target->abi != ROCK_DESCRIPTOR_ABI || target->pointer_bits != 64 ||
        target->little_endian != 1 || *(const unsigned char *)&endian != 1 ||
        target->index_limit != INT64_MAX || !target->key || !target->key_size || target->key_size > KEY_LIMIT)
        return ROCK_METADATA_TARGET;
    if (target->key_size < 20 || memcmp(target->key, "RT01", 4)) return ROCK_METADATA_TARGET;
    const unsigned char *p = target->key + 4, *triple, *layout_key;
    uint64_t left = target->key_size - 4, triple_len, layout_len;
    if (!take_frame(&p, &left, &triple, &triple_len) || !take_frame(&p, &left, &layout_key, &layout_len) ||
        left || !triple_len || !layout_len || layout_key[0] != 'e') return ROCK_METADATA_TARGET;
    int linux_target = 0;
    for (uint64_t i = 0; i + 5 <= triple_len; ++i) if (!memcmp(triple + i, "linux", 5)) linux_target = 1;
    if (!linux_target) return ROCK_METADATA_TARGET;
    return ROCK_METADATA_OK;
}
typedef struct { const unsigned char *bytes; uint64_t size; } KeyView;
static int template_header(const unsigned char **p, uint64_t *left, KeyView *origin,
    uint64_t *arguments, uint64_t *fields) {
    if (!*p || *left < 28 || *left > KEY_LIMIT || memcmp(*p, "RN01", 4)) return 0;
    *p += 4; *left -= 4;
    return take_frame(p, left, &origin->bytes, &origin->size) && valid_definition(origin->bytes, origin->size) &&
        take_word(p, left, arguments) && *arguments <= KEY_LIMIT / 8 &&
        take_word(p, left, fields) && *fields <= *left / 16;
}
static int valid_field_expression(const unsigned char *p, uint64_t left, uint64_t arguments, unsigned depth) {
    if (!p || left < 21 || left > KEY_LIMIT || depth >= DEPTH_LIMIT || memcmp(p, "RF01", 4)) return 0;
    unsigned char tag = p[4]; p += 5; left -= 5;
    const unsigned char *payload, *child; uint64_t n, count, child_len;
    if (!take_frame(&p, &left, &payload, &n) || !take_word(&p, &left, &count) || count > left / 8) return 0;
    switch (tag) {
        case 'v': if (n != 8 || count || read_word(payload) >= arguments) return 0; break;
        case 'c': if (count || !valid_key(payload, n, "RK01", depth + 1)) return 0; break;
        case 't': if (n) return 0; break;
        case 'S': if (!valid_definition(payload, n)) return 0; break;
        default: return 0;
    }
    for (uint64_t i = 0; i < count; ++i)
        if (!take_frame(&p, &left, &child, &child_len) || !valid_field_expression(child, child_len, arguments, depth + 1)) return 0;
    return left == 0;
}
static int valid_nominal_template(const unsigned char *p, uint64_t left) {
    KeyView origin; uint64_t arguments, fields;
    if (!template_header(&p, &left, &origin, &arguments, &fields)) return 0;
    const unsigned char *start = p;
    for (uint64_t i = 0; i < fields; ++i) {
        uint64_t id, n; const unsigned char *expression;
        if (!take_word(&p, &left, &id) || id > UINT32_MAX || !take_frame(&p, &left, &expression, &n) ||
            !valid_field_expression(expression, n, arguments, 0)) return 0;
        const unsigned char *previous = start; uint64_t remaining = (uint64_t)(p - start);
        for (uint64_t j = 0; j < i; ++j) {
            uint64_t other;
            if (!take_word(&previous, &remaining, &other) || other == id || !take_frame(&previous, &remaining, &expression, &n)) return 0;
        }
    }
    return left == 0;
}
static RockMetadataStatus encode_record(const char *magic, unsigned char tag, KeyView payload,
    uint64_t count, const KeyView *children, unsigned char **out, uint64_t *length) {
    if (payload.size > KEY_LIMIT - 21) return ROCK_METADATA_OVERFLOW;
    uint64_t size = 21 + payload.size;
    for (uint64_t i = 0; i < count; ++i) {
        if (size > KEY_LIMIT - 8 || children[i].size > KEY_LIMIT - size - 8) return ROCK_METADATA_OVERFLOW;
        size += 8 + children[i].size;
    }
    unsigned char *key = malloc((size_t)size);
    if (!key) return ROCK_METADATA_NOMEM;
    memcpy(key, magic, 4); key[4] = tag; write_word(key + 5, payload.size);
    if (payload.size) memcpy(key + 13, payload.bytes, (size_t)payload.size);
    write_word(key + 13 + payload.size, count);
    unsigned char *cursor = key + 21 + payload.size;
    for (uint64_t i = 0; i < count; ++i) {
        write_word(cursor, children[i].size); cursor += 8;
        memcpy(cursor, children[i].bytes, (size_t)children[i].size); cursor += children[i].size;
    }
    *out = key; *length = size; return ROCK_METADATA_OK;
}
static RockMetadataStatus evaluate_field(const unsigned char *p, uint64_t left,
    uint64_t argument_count, const KeyView *arguments, unsigned depth, unsigned char **out, uint64_t *length) {
    if (!valid_field_expression(p, left, argument_count, depth)) return ROCK_METADATA_INVALID;
    unsigned char tag = p[4]; p += 5; left -= 5;
    KeyView payload; uint64_t count;
    if (!take_frame(&p, &left, &payload.bytes, &payload.size) || !take_word(&p, &left, &count)) return ROCK_METADATA_INVALID;
    if (tag == 'v') payload = arguments[read_word(payload.bytes)];
    if (tag == 'v' || tag == 'c') {
        unsigned char *copy = malloc((size_t)payload.size);
        if (!copy) return ROCK_METADATA_NOMEM;
        memcpy(copy, payload.bytes, (size_t)payload.size); *out = copy; *length = payload.size; return ROCK_METADATA_OK;
    }
    if (count > SIZE_MAX / sizeof(KeyView)) return ROCK_METADATA_OVERFLOW;
    KeyView *children = calloc((size_t)(count ? count : 1), sizeof(*children));
    if (!children) return ROCK_METADATA_NOMEM;
    RockMetadataStatus status = ROCK_METADATA_OK;
    for (uint64_t i = 0; i < count; ++i) {
        KeyView expression; unsigned char *key = NULL;
        if (!take_frame(&p, &left, &expression.bytes, &expression.size)) { status = ROCK_METADATA_INVALID; break; }
        status = evaluate_field(expression.bytes, expression.size, argument_count, arguments, depth + 1, &key, &children[i].size);
        children[i].bytes = key; if (status != ROCK_METADATA_OK) break;
    }
    if (status == ROCK_METADATA_OK) status = encode_record("RK01", tag, payload, count, children, out, length);
    for (uint64_t i = 0; i < count; ++i) free((void *)children[i].bytes);
    free(children); return status;
}
static int nominal_binding(const RockDescriptor *d, const unsigned char *template_key, uint64_t template_len) {
    const unsigned char *p = d->key + 5; uint64_t left = d->key_size - 5, arguments, fields;
    KeyView semantic_origin, origin;
    if (d->key[4] != 'S' || !take_frame(&p, &left, &semantic_origin.bytes, &semantic_origin.size) || !take_word(&p, &left, &arguments) || arguments > SIZE_MAX / sizeof(KeyView)) return 0;
    KeyView *keys = calloc((size_t)(arguments ? arguments : 1), sizeof(*keys));
    if (!keys) return 0;
    int valid = 1;
    for (uint64_t i = 0; i < arguments; ++i) if (!take_frame(&p, &left, &keys[i].bytes, &keys[i].size)) valid = 0;
    uint64_t expected_arguments; p = template_key; left = template_len;
    if (!template_header(&p, &left, &origin, &expected_arguments, &fields) || expected_arguments != arguments || fields != d->child_count ||
        !equal_bytes(origin.size, origin.bytes, semantic_origin.size, semantic_origin.bytes)) valid = 0;
    if (valid) for (uint64_t i = 0; i < fields; ++i) {
        uint64_t id, n = 0; KeyView expression; unsigned char *key = NULL;
        if (!take_word(&p, &left, &id) || !take_frame(&p, &left, &expression.bytes, &expression.size) ||
            evaluate_field(expression.bytes, expression.size, arguments, keys, 0, &key, &n) != ROCK_METADATA_OK ||
            !d->children[i] || !equal_bytes(n, key, d->children[i]->key_size, d->children[i]->key)) valid = 0;
        free(key); if (!valid) break;
    }
    free(keys); return valid;
}
static int drop_binding(const RockDescriptor *d) {
    if (d->key[4] == 't') return d->kind == ROCK_DESCRIPTOR_TUPLE && d->drop_key[4] == 't' &&
        read_word(d->key + 13) == d->child_count && read_word(d->drop_key + 13) == d->child_count;
    if (d->kind != ROCK_DESCRIPTOR_NATIVE || d->drop_key[4] == 't') return 0;
    const unsigned char *p = d->drop_key + 5, *payload, *semantic, *child;
    uint64_t left = d->drop_key_size - 5, payload_len, n, count;
    if (!take_frame(&p, &left, &payload, &payload_len) || !take_word(&p, &left, &count)) return 0;
    const unsigned char *q = payload; uint64_t remaining = payload_len;
    if (!take_frame(&q, &remaining, &semantic, &n) || !equal_bytes(n, semantic, d->key_size, d->key)) return 0;
    if (d->drop_key[4] == 'g') {
        const unsigned char *template_key; uint64_t template_len;
        if (!remaining || *q++ != 0) return 0;
        --remaining;
        if (!take_frame(&q, &remaining, &template_key, &template_len) || remaining ||
            !nominal_binding(d, template_key, template_len)) return 0;
    }
    if (d->drop_key[4] == 'f' || d->drop_key[4] == 'g') {
        if (count != d->child_count) return 0;
        for (uint64_t i = 0; i < count; ++i) {
            if (!d->children[i] || !take_frame(&p, &left, &child, &n) ||
                !equal_bytes(n, child, d->children[i]->drop_key_size, d->children[i]->drop_key)) return 0;
        }
    } else if (d->child_count) return 0;
    return 1;
}
static RockMetadataStatus layout(uint64_t count, const RockDescriptor *const *children,
    uint64_t limit, uint64_t *offsets, uint64_t *size, uint64_t *alignment) {
    uint64_t cursor = 0, align = 1;
    for (uint64_t i = 0; i < count; ++i) {
        uint64_t a = children[i]->alignment, n = children[i]->size;
        if (!a || (a & (a - 1))) return ROCK_METADATA_INVALID;
        if (a > limit || cursor > limit - (a - 1)) return ROCK_METADATA_OVERFLOW;
        cursor = (cursor + a - 1) & ~(a - 1);
        offsets[i] = cursor;
        if (n > limit - cursor) return ROCK_METADATA_OVERFLOW;
        cursor += n; if (a > align) align = a;
    }
    if (cursor > limit - (align - 1)) return ROCK_METADATA_OVERFLOW;
    cursor = (cursor + align - 1) & ~(align - 1);
    /* Storage allocators need size + alignment - 1 as well. */
    if (cursor > limit - (align - 1)) return ROCK_METADATA_OVERFLOW;
    *size = cursor; *alignment = align; return ROCK_METADATA_OK;
}
static RockMetadataStatus compose_key(const char *magic, uint64_t count,
    const RockDescriptor *const *children, int drop, unsigned char **out, uint64_t *length) {
    if (count > SIZE_MAX / sizeof(KeyView)) return ROCK_METADATA_OVERFLOW;
    KeyView *keys = calloc((size_t)(count ? count : 1), sizeof(*keys));
    if (!keys) return ROCK_METADATA_NOMEM;
    for (uint64_t i = 0; i < count; ++i) {
        keys[i].bytes = drop ? children[i]->drop_key : children[i]->key;
        keys[i].size = drop ? children[i]->drop_key_size : children[i]->key_size;
    }
    RockMetadataStatus status = encode_record(magic, 't', (KeyView){NULL, 0}, count, keys, out, length);
    free(keys); return status;
}
static void release_candidate(RockDescriptor *d) {
    free((void *)d->key); free((void *)d->drop_key); free((void *)d->offsets);
    free((void *)d->children); free((void *)d->target_key);
}
static int same_descriptor(const RockDescriptor *a, const RockDescriptor *b) {
    if (a->size != b->size || a->alignment != b->alignment || a->kind != b->kind ||
        a->field_count != b->field_count || a->child_count != b->child_count || a->index_limit != b->index_limit ||
        !equal_bytes(a->drop_key_size, a->drop_key, b->drop_key_size, b->drop_key) ||
        !equal_bytes(a->target_key_size, a->target_key, b->target_key_size, b->target_key)) return 0;
    for (uint64_t i = 0; i < a->field_count; ++i) if (a->offsets[i] != b->offsets[i]) return 0;
    for (uint64_t i = 0; i < a->child_count; ++i)
        if (!equal_bytes(a->children[i]->key_size, a->children[i]->key, b->children[i]->key_size, b->children[i]->key)) return 0;
    return 1;
}
static RockMetadataStatus publish(RockDescriptor *candidate, const RockDescriptor **result) {
    uint64_t hash = key_hash(candidate->key, candidate->key_size);
    for (Record *r = records; r; r = r->next) {
        if (r->hash != hash || !equal_bytes(r->descriptor.key_size, r->descriptor.key, candidate->key_size, candidate->key)) continue;
        RockMetadataStatus status = r->state != ROCK_METADATA_READY ? ROCK_METADATA_CYCLE :
            same_descriptor(&r->descriptor, candidate) ? ROCK_METADATA_OK : ROCK_METADATA_CONFLICT;
        if (status == ROCK_METADATA_OK) *result = &r->descriptor;
        release_candidate(candidate); return status;
    }
    Record *record = calloc(1, sizeof(*record));
    if (!record) { release_candidate(candidate); return ROCK_METADATA_NOMEM; }
    record->hash = hash; record->state = ROCK_METADATA_BUILDING;
    record->descriptor = *candidate; record->descriptor.state = ROCK_METADATA_BUILDING;
    record->next = records; records = record;
    /* No callback/factory recursion occurs under this mutex. Children must
     * already be complete. Building never escapes; Failed is fail-closed. */
    if (!record->descriptor.drop) {
        record->state = ROCK_METADATA_FAILED; record->descriptor.state = ROCK_METADATA_FAILED;
        return ROCK_METADATA_INVALID;
    }
    record->descriptor.state = ROCK_METADATA_READY; record->state = ROCK_METADATA_READY;
    *result = &record->descriptor; return ROCK_METADATA_OK;
}
static unsigned char *copy_bytes(const unsigned char *bytes, uint64_t len) {
    unsigned char *copy = malloc((size_t)(len ? len : 1));
    if (copy && len) memcpy(copy, bytes, (size_t)len);
    return copy;
}
static RockMetadataStatus freeze(const RockDescriptorTarget *target, const RockDescriptor *input,
    const RockDescriptor **active, unsigned depth, const RockDescriptor **result) {
    if (!input || depth >= DEPTH_LIMIT) return ROCK_METADATA_CYCLE;
    for (unsigned i = 0; i < depth; ++i) if (active[i] == input) return ROCK_METADATA_CYCLE;
    if (input->state == ROCK_METADATA_BUILDING) return ROCK_METADATA_CYCLE;
    if (input->state != ROCK_METADATA_READY || input->abi != ROCK_DESCRIPTOR_ABI || !input->drop ||
        (input->kind != ROCK_DESCRIPTOR_NATIVE && input->kind != ROCK_DESCRIPTOR_TUPLE) ||
        !valid_key(input->key, input->key_size, "RK01", 0) || !valid_key(input->drop_key, input->drop_key_size, "RD01", 0) ||
        !equal_bytes(input->target_key_size, input->target_key, target->key_size, target->key) || input->index_limit != target->index_limit)
        return ROCK_METADATA_INVALID;
    if (!input->alignment || (input->alignment & (input->alignment - 1))) return ROCK_METADATA_INVALID;
    if (input->alignment > target->index_limit || input->size > target->index_limit - (input->alignment - 1)) return ROCK_METADATA_OVERFLOW;
    if (input->field_count != input->child_count || input->field_count > SIZE_MAX / sizeof(uint64_t) ||
        (input->field_count && (!input->offsets || !input->children))) return ROCK_METADATA_INVALID;
    if (!drop_binding(input)) return ROCK_METADATA_INVALID;
    RockDescriptor d = *input;
    d.key = copy_bytes(input->key, input->key_size); d.drop_key = copy_bytes(input->drop_key, input->drop_key_size);
    d.target_key = copy_bytes(input->target_key, input->target_key_size);
    uint64_t *offsets = calloc((size_t)(input->field_count ? input->field_count : 1), sizeof(*offsets));
    const RockDescriptor **children = calloc((size_t)(input->child_count ? input->child_count : 1), sizeof(*children));
    d.offsets = offsets; d.children = children;
    if (!d.key || !d.drop_key || !d.target_key || !offsets || !children) { release_candidate(&d); return ROCK_METADATA_NOMEM; }
    active[depth] = input;
    RockMetadataStatus status = ROCK_METADATA_OK;
    for (uint64_t i = 0; i < input->child_count; ++i) {
        status = freeze(target, input->children[i], active, depth + 1, &children[i]);
        if (status != ROCK_METADATA_OK) break;
        offsets[i] = input->offsets[i];
        if (offsets[i] > input->size || children[i]->size > input->size - offsets[i] || offsets[i] % children[i]->alignment) { status = ROCK_METADATA_INVALID; break; }
    }
    if (status == ROCK_METADATA_OK && input->kind == ROCK_DESCRIPTOR_TUPLE) {
        unsigned char *key = NULL, *drop_key = NULL; uint64_t n = 0, dn = 0;
        status = compose_key("RK01", d.child_count, children, 0, &key, &n);
        if (status == ROCK_METADATA_OK) status = compose_key("RD01", d.child_count, children, 1, &drop_key, &dn);
        if (status == ROCK_METADATA_OK && (!equal_bytes(n, key, d.key_size, d.key) || !equal_bytes(dn, drop_key, d.drop_key_size, d.drop_key))) status = ROCK_METADATA_INVALID;
        free(key); free(drop_key);
    }
    if (status == ROCK_METADATA_OK && (input->kind == ROCK_DESCRIPTOR_TUPLE || input->drop_key[4] == 'g')) {
            uint64_t size = 0, align = 0;
            uint64_t *expected = calloc((size_t)(d.child_count ? d.child_count : 1), sizeof(*expected));
            if (!expected) status = ROCK_METADATA_NOMEM;
            else {
                status = layout(d.child_count, children, target->index_limit, expected, &size, &align);
                if (status == ROCK_METADATA_OK && (size != d.size || align != d.alignment || memcmp(expected, offsets, (size_t)d.child_count * sizeof(*offsets)))) status = ROCK_METADATA_CONFLICT;
                free(expected);
            }
    }
    if (status != ROCK_METADATA_OK) { release_candidate(&d); return status; }
    return publish(&d, result);
}
void rock_descriptor_tuple_drop(void *payload, const RockDescriptor *descriptor) {
    /* Only the checked factory publishes this destructor/environment pair. */
    for (uint64_t i = descriptor->child_count; i > 0; --i) {
        const RockDescriptor *child = descriptor->children[i - 1];
        child->drop((unsigned char *)payload + descriptor->offsets[i - 1], child);
    }
}
static RockMetadataStatus record_factory(const RockDescriptorTarget *target, const RockNominalTemplate *template,
    uint64_t count, const RockDescriptor *const *arguments, const RockDescriptor *const *inputs, const RockDescriptor **result) {
    if (!result) return ROCK_METADATA_INVALID;
    *result = NULL;
    RockMetadataStatus status = target_valid(target);
    if (status != ROCK_METADATA_OK) return status;
    KeyView origin = {NULL, 0}; uint64_t argument_count = 0;
    if (template) {
        const unsigned char *p = template->key; uint64_t left = template->key_size;
        if (template->abi != ROCK_DESCRIPTOR_ABI || !valid_nominal_template(p, left) ||
            !template_header(&p, &left, &origin, &argument_count, &count) || (argument_count && !arguments)) return ROCK_METADATA_INVALID;
    }
    if ((count && !inputs) || count > SIZE_MAX / sizeof(void *) || count > KEY_LIMIT / 8) return ROCK_METADATA_INVALID;
    RockDescriptor d = {0}; d.abi = ROCK_DESCRIPTOR_ABI; d.kind = template ? ROCK_DESCRIPTOR_NATIVE : ROCK_DESCRIPTOR_TUPLE;
    d.state = ROCK_METADATA_READY; d.field_count = d.child_count = count;
    d.drop = rock_descriptor_tuple_drop; d.index_limit = target->index_limit; d.target_key_size = target->key_size;
    d.target_key = copy_bytes(target->key, target->key_size);
    uint64_t *offsets = calloc((size_t)(count ? count : 1), sizeof(*offsets));
    const RockDescriptor **children = calloc((size_t)(count ? count : 1), sizeof(*children));
    KeyView *argument_keys = calloc((size_t)(argument_count ? argument_count : 1), sizeof(*argument_keys));
    d.offsets = offsets; d.children = children;
    if (!offsets || !children || !d.target_key || !argument_keys) { free(argument_keys); release_candidate(&d); return ROCK_METADATA_NOMEM; }
    if (pthread_mutex_lock(&metadata_mutex)) { free(argument_keys); release_candidate(&d); return ROCK_METADATA_INVALID; }
    const RockDescriptor *active[DEPTH_LIMIT];
    for (uint64_t i = 0; i < argument_count; ++i) {
        const RockDescriptor *argument = NULL;
        status = freeze(target, arguments[i], active, 0, &argument);
        if (status != ROCK_METADATA_OK) break;
        argument_keys[i] = (KeyView){argument->key, argument->key_size};
    }
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < count; ++i) {
        status = freeze(target, inputs[i], active, 0, &children[i]);
        if (status != ROCK_METADATA_OK) break;
    }
    if (status == ROCK_METADATA_OK) status = layout(count, children, target->index_limit, offsets, &d.size, &d.alignment);
    unsigned char *key = NULL, *drop_key = NULL;
    if (status == ROCK_METADATA_OK) status = template
        ? encode_record("RK01", 'S', origin, argument_count, argument_keys, &key, &d.key_size)
        : compose_key("RK01", count, children, 0, &key, &d.key_size);
    d.key = key;
    if (status == ROCK_METADATA_OK && template) {
        if (d.key_size > KEY_LIMIT - 17 || template->key_size > KEY_LIMIT - 17 - d.key_size) status = ROCK_METADATA_OVERFLOW;
        else {
            uint64_t n = 17 + d.key_size + template->key_size;
            unsigned char *payload = malloc((size_t)n);
            KeyView *drops = calloc((size_t)(count ? count : 1), sizeof(*drops));
            if (!payload || !drops) status = ROCK_METADATA_NOMEM;
            else {
                write_word(payload, d.key_size); memcpy(payload + 8, d.key, (size_t)d.key_size);
                payload[8 + d.key_size] = 0; write_word(payload + 9 + d.key_size, template->key_size);
                memcpy(payload + 17 + d.key_size, template->key, (size_t)template->key_size);
                for (uint64_t i = 0; i < count; ++i) drops[i] = (KeyView){children[i]->drop_key, children[i]->drop_key_size};
                status = encode_record("RD01", 'g', (KeyView){payload, n}, count, drops, &drop_key, &d.drop_key_size);
            }
            free(payload); free(drops);
        }
    } else if (status == ROCK_METADATA_OK) status = compose_key("RD01", count, children, 1, &drop_key, &d.drop_key_size);
    d.drop_key = drop_key;
    if (status == ROCK_METADATA_OK && template && !nominal_binding(&d, template->key, template->key_size)) status = ROCK_METADATA_INVALID;
    if (status == ROCK_METADATA_OK) status = publish(&d, result);
    else release_candidate(&d);
    pthread_mutex_unlock(&metadata_mutex);
    free(argument_keys);
    return status;
}

RockMetadataStatus rock_descriptor_tuple(const RockDescriptorTarget *target, uint64_t count,
    const RockDescriptor *const *inputs, const RockDescriptor **result) {
    return record_factory(target, NULL, count, NULL, inputs, result);
}
RockMetadataStatus rock_descriptor_nominal(const RockDescriptorTarget *target, const RockNominalTemplate *template,
    const RockDescriptor *const *arguments, const RockDescriptor *const *fields, const RockDescriptor **result) {
    if (!template) { if (result) *result = NULL; return ROCK_METADATA_INVALID; }
    return record_factory(target, template, 0, arguments, fields, result);
}

/* Dictionaries have their own exact-identity namespace, under the same mutex.
 * The registry retains immutable metadata only, never receiver/payload values. */
_Static_assert(sizeof(RockDictionary) == 128 && sizeof(RockDictionaryTemplate) == 40, "dictionary ABI mismatch");
_Static_assert(offsetof(RockDictionary, entries) == 64 && offsetof(RockDictionary, types) == 80 &&
    offsetof(RockDictionary, children) == 96, "dictionary environment ABI offsets mismatch");
typedef struct DictionaryRecord {
    struct DictionaryRecord *next;
    uint64_t namespace_kind, hash, state;
    RockDictionary dictionary;
} DictionaryRecord;
static DictionaryRecord *dictionary_records;
typedef struct { unsigned char *bytes; uint64_t size; } DictionaryBuffer;
static RockMetadataStatus db_append(DictionaryBuffer *b, const unsigned char *p, uint64_t n) {
    if (n > KEY_LIMIT - b->size) return ROCK_METADATA_OVERFLOW;
    unsigned char *bytes = realloc(b->bytes, (size_t)(b->size + n ? b->size + n : 1));
    if (!bytes) return ROCK_METADATA_NOMEM;
    b->bytes = bytes;
    if (n) memcpy(bytes + b->size, p, (size_t)n);
    b->size += n; return ROCK_METADATA_OK;
}
static RockMetadataStatus db_word(DictionaryBuffer *b, uint64_t n) {
    unsigned char word[8]; write_word(word, n); return db_append(b, word, 8);
}
static RockMetadataStatus db_frame(DictionaryBuffer *b, const unsigned char *p, uint64_t n) {
    RockMetadataStatus s = db_word(b, n); return s == ROCK_METADATA_OK ? db_append(b, p, n) : s;
}
/* BQ01 carries the complete conservative origin envelope, even for scalar and
 * unit results. It grants no loans. Re-derive it from the exact signature both
 * before and after capture substitution; returned references/retention remain
 * outside this factory, rather than being replaced by an empty contract. */
typedef struct {
    uint64_t tags[DEPTH_LIMIT], indices[DEPTH_LIMIT], count;
} DictionaryBorrowPath;
typedef struct {
    DictionaryBuffer sources, returns;
    uint64_t source_count, return_count;
} DictionaryBorrowShape;
static RockMetadataStatus borrow_path(DictionaryBuffer *out, const DictionaryBorrowPath *path) {
    unsigned char bytes[8 + DEPTH_LIMIT * 16]; uint64_t n = 8;
    write_word(bytes, path->count);
    for (uint64_t i = 0; i < path->count; ++i) {
        write_word(bytes + n, path->tags[i]); n += 8;
        if (path->tags[i] == 0) { write_word(bytes + n, path->indices[i]); n += 8; }
    }
    return db_frame(out, bytes, n);
}
static RockMetadataStatus borrow_source(DictionaryBorrowShape *shape, uint64_t argument,
    uint64_t kind, const DictionaryBorrowPath *path) {
    RockMetadataStatus status = db_word(&shape->sources, argument);
    if (status == ROCK_METADATA_OK) status = db_word(&shape->sources, kind);
    if (status == ROCK_METADATA_OK) status = borrow_path(&shape->sources, path);
    if (status == ROCK_METADATA_OK) ++shape->source_count;
    return status;
}
static RockMetadataStatus borrow_walk(KeyView type, int concrete, int output, uint64_t argument,
    DictionaryBorrowPath *path, DictionaryBorrowShape *shape, uint64_t captures, unsigned depth) {
    if (depth >= DEPTH_LIMIT || path->count >= DEPTH_LIMIT) return ROCK_METADATA_INVALID;
    const unsigned char *p = type.bytes, *payload; uint64_t left = type.size, n, count;
    if (concrete ? !valid_key(p, left, "RK01", depth) : !valid_field_expression(p, left, captures, depth)) return ROCK_METADATA_INVALID;
    unsigned char tag = p[4]; p += 5; left -= 5;
    if (!take_frame(&p, &left, &payload, &n) || !take_word(&p, &left, &count)) return ROCK_METADATA_INVALID;
    if (!concrete && tag == 'c') return borrow_walk((KeyView){payload, n}, 1, output, argument, path, shape, captures, depth + 1);
    if (tag == 'p') return ROCK_METADATA_OK; /* raw pointers carry no loans */
    if (tag == 't' || (concrete && tag == 'A')) {
        for (uint64_t i = 0; i < count; ++i) {
            KeyView child;
            if (!take_frame(&p, &left, &child.bytes, &child.size)) return ROCK_METADATA_INVALID;
            path->tags[path->count] = tag == 't' ? 0 : 1;
            path->indices[path->count++] = i;
            RockMetadataStatus status = borrow_walk(child, concrete, output, argument, path, shape, captures, depth + 1);
            --path->count;
            if (status != ROCK_METADATA_OK) return status;
        }
        return ROCK_METADATA_OK;
    }
    if (output) {
        /* Only primitive leaves are proved borrow-free here. Opaque nominal,
         * parameter, reference and callable results require other authority. */
        if (!concrete || tag != 'a') return ROCK_METADATA_INVALID;
        RockMetadataStatus status = borrow_path(&shape->returns, path);
        if (status == ROCK_METADATA_OK) status = db_word(&shape->returns, shape->source_count);
        if (status == ROCK_METADATA_OK) status = db_append(&shape->returns, shape->sources.bytes, shape->sources.size);
        if (status == ROCK_METADATA_OK) ++shape->return_count;
        return status;
    }
    /* These shapes have declaration retention destinations. No capture list,
     * nominal layout or runtime key can erase that permission obligation. */
    if (tag == 'R' || (argument != 0 && (tag == 'S' || tag == 'E' || tag == 'v'))) return ROCK_METADATA_INVALID;
    RockMetadataStatus status = borrow_source(shape, argument, 0, path);
    if (status == ROCK_METADATA_OK && concrete && tag == 'r') status = borrow_source(shape, argument, 1, path);
    return status;
}
static RockMetadataStatus dictionary_borrow(const KeyView *params, uint64_t count, KeyView ret,
    int concrete, uint64_t captures, uint64_t receiver, DictionaryBuffer *out) {
    if (receiver != 0) return ROCK_METADATA_INVALID;
    DictionaryBorrowShape shape = {{NULL, 0}, {NULL, 0}, 0, 0};
    DictionaryBorrowPath path = {{0}, {0}, 0}; RockMetadataStatus status = ROCK_METADATA_OK;
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < count; ++i)
        status = borrow_walk(params[i], concrete, 0, i, &path, &shape, captures, 0);
    if (status == ROCK_METADATA_OK) status = borrow_walk(ret, concrete, 1, 0, &path, &shape, captures, 0);
    if (status == ROCK_METADATA_OK) status = db_append(out, (const unsigned char *)"BQ01", 4);
    if (status == ROCK_METADATA_OK) status = db_word(out, shape.return_count);
    if (status == ROCK_METADATA_OK) status = db_append(out, shape.returns.bytes, shape.returns.size);
    if (status == ROCK_METADATA_OK) status = db_word(out, 0); /* no retention effects */
    free(shape.sources.bytes); free(shape.returns.bytes); return status;
}
/* Static Self is opaque, not borrow-free. Its canonical envelope contains every
 * real scalar argument and no receiver-frame source. Native body checks, not
 * this metadata key, establish that a concrete leaf can initialize Self. */
static int static_scalar_argument(KeyView type, int concrete, uint64_t captures) {
    const unsigned char *p = type.bytes, *payload; uint64_t left = type.size, n, children;
    if (!concrete) {
        if (!valid_field_expression(p, left, captures, 0) || p[4] != 'c') return 0;
        p += 5; left -= 5;
        if (!take_frame(&p, &left, &payload, &n) || !take_word(&p, &left, &children) || children || left) return 0;
        p = payload; left = n;
    }
    if (!valid_key(p, left, "RK01", 0) || p[4] != 'a') return 0;
    p += 5; left -= 5;
    if (!take_frame(&p, &left, &payload, &n) || !take_word(&p, &left, &children) || children || left) return 0;
    return !equal_bytes(n, payload, 3, (const unsigned char *)"str") && !equal_bytes(n, payload, 5, (const unsigned char *)"never");
}
static RockMetadataStatus static_dictionary_borrow(const KeyView *params, uint64_t count,
    KeyView ret, KeyView subject, int concrete, uint64_t captures, DictionaryBuffer *out) {
    if (!equal_bytes(ret.size, ret.bytes, subject.size, subject.bytes)) return ROCK_METADATA_INVALID;
    if (concrete && (!valid_key(subject.bytes, subject.size, "RK01", 0)
        || (subject.bytes[4] != 'a' && subject.bytes[4] != 'S' && subject.bytes[4] != 'E'))) return ROCK_METADATA_INVALID;
    DictionaryBorrowShape shape = {{NULL, 0}, {NULL, 0}, 0, 0};
    DictionaryBorrowPath path = {{0}, {0}, 0}; RockMetadataStatus status = ROCK_METADATA_OK;
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < count; ++i) {
        if (!static_scalar_argument(params[i], concrete, captures)) { status = ROCK_METADATA_INVALID; break; }
        status = borrow_source(&shape, i, 0, &path);
    }
    if (status == ROCK_METADATA_OK) status = db_append(out, (const unsigned char *)"BQ01", 4);
    if (status == ROCK_METADATA_OK) status = db_word(out, 1);
    if (status == ROCK_METADATA_OK) status = borrow_path(out, &path);
    if (status == ROCK_METADATA_OK) status = db_word(out, shape.source_count);
    if (status == ROCK_METADATA_OK) status = db_append(out, shape.sources.bytes, shape.sources.size);
    if (status == ROCK_METADATA_OK) status = db_word(out, 0);
    free(shape.sources.bytes); return status;
}
static RockMetadataStatus schema_type(DictionaryBuffer *b, KeyView expression, int concrete,
    uint64_t count, const KeyView *arguments, KeyView *value) {
    unsigned char *key = NULL; uint64_t n = 0;
    RockMetadataStatus status;
    if (concrete) {
        if (!valid_key(expression.bytes, expression.size, "RK01", 0)) return ROCK_METADATA_INVALID;
        key = copy_bytes(expression.bytes, expression.size); n = expression.size;
        status = key ? ROCK_METADATA_OK : ROCK_METADATA_NOMEM;
    } else status = evaluate_field(expression.bytes, expression.size, count, arguments, 0, &key, &n);
    if (status == ROCK_METADATA_OK) status = db_frame(b, key, n);
    if (value && status == ROCK_METADATA_OK) { value->bytes = key; value->size = n; }
    else free(key);
    return status;
}
/* PM01 distinguishes a member-local variable from a parent expression. Open
 * locals are never evaluated with captured descriptors or stored per call. */
static int member_type_view(KeyView input, uint64_t locals, unsigned char *tag, KeyView *payload) {
    const unsigned char *p = input.bytes; uint64_t left = input.size, children;
    if (!p || left < 5 || left > KEY_LIMIT || memcmp(p, "PM01", 4)) return 0;
    *tag = p[4]; p += 5; left -= 5;
    if (!take_frame(&p, &left, &payload->bytes, &payload->size) || !take_word(&p, &left, &children) || children || left) return 0;
    if (*tag == 'm') return payload->size == 8 && read_word(payload->bytes) < locals;
    return *tag == 'c';
}
static RockMetadataStatus member_schema_type(DictionaryBuffer *b, KeyView expression, int concrete,
    uint64_t captures, const KeyView *arguments, uint64_t locals, KeyView *value) {
    unsigned char tag; KeyView payload;
    if (!member_type_view(expression, locals, &tag, &payload)) return ROCK_METADATA_INVALID;
    unsigned char *evaluated = NULL, *key = NULL; uint64_t n = 0, key_size = 0;
    RockMetadataStatus status = ROCK_METADATA_OK;
    if (tag == 'c') {
        if (concrete) {
            if (!valid_key(payload.bytes, payload.size, "RK01", 0)) return ROCK_METADATA_INVALID;
        } else {
            status = evaluate_field(payload.bytes, payload.size, captures, arguments, 0, &evaluated, &n);
            if (status == ROCK_METADATA_OK) payload = (KeyView){evaluated, n};
        }
    }
    if (status == ROCK_METADATA_OK) status = encode_record("PM01", tag, payload, 0, NULL, &key, &key_size);
    if (status == ROCK_METADATA_OK) status = db_frame(b, key, key_size);
    if (value && status == ROCK_METADATA_OK) *value = (KeyView){key, key_size}; else free(key);
    free(evaluated); return status;
}
static RockMetadataStatus member_borrow(const KeyView *params, uint64_t count, KeyView ret,
    uint64_t locals, uint64_t receiver, DictionaryBuffer *out) {
    unsigned char tag; KeyView result, payload;
    if (receiver || count < 2 || !member_type_view(ret, locals, &tag, &result) || tag != 'm') return ROCK_METADATA_INVALID;
    DictionaryBorrowShape shape = {{NULL, 0}, {NULL, 0}, 0, 0}; DictionaryBorrowPath path = {{0}, {0}, 0};
    RockMetadataStatus status = ROCK_METADATA_OK;
    for (uint64_t i = 1; status == ROCK_METADATA_OK && i < count; ++i) {
        if (!member_type_view(params[i], locals, &tag, &payload) || tag != 'm') { status = ROCK_METADATA_INVALID; break; }
        if (read_word(payload.bytes) == read_word(result.bytes)) status = borrow_source(&shape, i, 0, &path);
    }
    if (!shape.source_count) status = ROCK_METADATA_INVALID;
    if (status == ROCK_METADATA_OK) status = db_append(out, (const unsigned char *)"BQ01", 4);
    if (status == ROCK_METADATA_OK) status = db_word(out, 1);
    if (status == ROCK_METADATA_OK) status = borrow_path(out, &path);
    if (status == ROCK_METADATA_OK) status = db_word(out, shape.source_count);
    if (status == ROCK_METADATA_OK) status = db_append(out, shape.sources.bytes, shape.sources.size);
    if (status == ROCK_METADATA_OK) status = db_word(out, 0);
    free(shape.sources.bytes); return status;
}
static RockMetadataStatus dictionary_schema(KeyView input, int concrete, uint64_t count,
    const KeyView *arguments, unsigned char **output, uint64_t *size, uint64_t *member_count) {
    const unsigned char *p = input.bytes; uint64_t left = input.size, abi, n = 0, members = 0;
    if (!p || left < 12 || left > KEY_LIMIT || memcmp(p, concrete ? "DK01" : "DS01", 4)) return ROCK_METADATA_INVALID;
    p += 4; left -= 4;
    if (!take_word(&p, &left, &abi) || abi != ROCK_DICTIONARY_ABI) return ROCK_METADATA_INVALID;
    KeyView origin = {NULL, 0}, expression = {NULL, 0}, subject = {NULL, 0}; DictionaryBuffer b = {NULL, 0};
    RockMetadataStatus status = db_append(&b, (const unsigned char *)"DK01", 4);
    if (status == ROCK_METADATA_OK) status = db_word(&b, abi);
    if (!take_frame(&p, &left, &origin.bytes, &origin.size) || !valid_definition(origin.bytes, origin.size)) status = ROCK_METADATA_INVALID;
    if (status == ROCK_METADATA_OK) status = db_frame(&b, origin.bytes, origin.size);
    if (!take_frame(&p, &left, &expression.bytes, &expression.size)) status = ROCK_METADATA_INVALID;
    KeyView original_subject = expression;
    if (status == ROCK_METADATA_OK) status = schema_type(&b, expression, concrete, count, arguments, &subject);
    if (!take_word(&p, &left, &n) || n > left / 8) status = ROCK_METADATA_INVALID;
    if (status == ROCK_METADATA_OK) status = db_word(&b, n);
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < n; ++i) {
        if (!take_frame(&p, &left, &expression.bytes, &expression.size)) { status = ROCK_METADATA_INVALID; break; }
        status = schema_type(&b, expression, concrete, count, arguments, NULL);
    }
    if (!take_word(&p, &left, &members) || members > left / 8 || members > SIZE_MAX / sizeof(KeyView)) status = ROCK_METADATA_INVALID;
    KeyView *ids = status == ROCK_METADATA_OK ? calloc((size_t)(members ? members : 1), sizeof(*ids)) : NULL;
    if (status == ROCK_METADATA_OK && !ids) status = ROCK_METADATA_NOMEM;
    if (status == ROCK_METADATA_OK) status = db_word(&b, members);
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < members; ++i) {
        uint64_t receiver, params, locals, bounds, kind;
        if (!take_frame(&p, &left, &ids[i].bytes, &ids[i].size) || !valid_definition(ids[i].bytes, ids[i].size) ||
            !take_word(&p, &left, &receiver) || receiver > 3 || !take_word(&p, &left, &locals) || locals > left / 8 || (receiver == 3 && locals)) { status = ROCK_METADATA_INVALID; break; }
        for (uint64_t j = 0; j < i; ++j) if (equal_bytes(ids[i].size, ids[i].bytes, ids[j].size, ids[j].bytes)) status = ROCK_METADATA_INVALID;
        if (status == ROCK_METADATA_OK) status = db_frame(&b, ids[i].bytes, ids[i].size);
        if (status == ROCK_METADATA_OK) status = db_word(&b, receiver);
        if (status == ROCK_METADATA_OK) status = db_word(&b, locals);
        for (uint64_t j = 0; status == ROCK_METADATA_OK && j < locals; ++j) {
            if (!take_word(&p, &left, &kind) || kind) { status = ROCK_METADATA_INVALID; break; }
            status = db_word(&b, kind);
        }
        if (!take_word(&p, &left, &bounds) || bounds || !take_word(&p, &left, &params) || (!params && receiver != 3) || params > left / 8
            || params > SIZE_MAX / sizeof(KeyView)) status = ROCK_METADATA_INVALID;
        if (status != ROCK_METADATA_OK) break;
        status = db_word(&b, bounds);
        if (status == ROCK_METADATA_OK) status = db_word(&b, params);
        KeyView *original = status == ROCK_METADATA_OK ? calloc((size_t)(params ? params : 1), sizeof(*original)) : NULL;
        KeyView *evaluated = status == ROCK_METADATA_OK ? calloc((size_t)(params ? params : 1), sizeof(*evaluated)) : NULL;
        if (status == ROCK_METADATA_OK && (!original || !evaluated)) status = ROCK_METADATA_NOMEM;
        for (uint64_t j = 0; status == ROCK_METADATA_OK && j < params; ++j) {
            if (!take_frame(&p, &left, &expression.bytes, &expression.size)) { status = ROCK_METADATA_INVALID; break; }
            original[j] = expression;
            status = locals ? member_schema_type(&b, expression, concrete, count, arguments, locals, &evaluated[j])
                : schema_type(&b, expression, concrete, count, arguments, &evaluated[j]);
            if (j == 0 && receiver != 3 && status == ROCK_METADATA_OK) {
                KeyView receiver_type = evaluated[j]; unsigned char tag;
                if (locals && (!member_type_view(evaluated[j], locals, &tag, &receiver_type) || tag != 'c')) status = ROCK_METADATA_INVALID;
                if (status == ROCK_METADATA_OK && !equal_bytes(receiver_type.size, receiver_type.bytes, subject.size, subject.bytes)) status = ROCK_METADATA_INVALID;
            }
        }
        KeyView original_ret = {NULL, 0}, evaluated_ret = {NULL, 0}, borrowing = {NULL, 0};
        if (!take_frame(&p, &left, &original_ret.bytes, &original_ret.size)) status = ROCK_METADATA_INVALID;
        if (status == ROCK_METADATA_OK) status = locals ? member_schema_type(&b, original_ret, concrete, count, arguments, locals, &evaluated_ret)
            : schema_type(&b, original_ret, concrete, count, arguments, &evaluated_ret);
        if (!take_frame(&p, &left, &borrowing.bytes, &borrowing.size)) status = ROCK_METADATA_INVALID;
        DictionaryBuffer declared = {NULL, 0}, normalized = {NULL, 0};
        if (status == ROCK_METADATA_OK) status = receiver == 3 ? static_dictionary_borrow(original, params, original_ret, original_subject, concrete, count, &declared)
            : locals ? member_borrow(original, params, original_ret, locals, receiver, &declared)
            : dictionary_borrow(original, params, original_ret, concrete, count, receiver, &declared);
        if (status == ROCK_METADATA_OK && !equal_bytes(declared.size, declared.bytes, borrowing.size, borrowing.bytes)) status = ROCK_METADATA_INVALID;
        if (status == ROCK_METADATA_OK) status = receiver == 3 ? static_dictionary_borrow(evaluated, params, evaluated_ret, subject, 1, count, &normalized)
            : locals ? member_borrow(evaluated, params, evaluated_ret, locals, receiver, &normalized)
            : dictionary_borrow(evaluated, params, evaluated_ret, 1, count, receiver, &normalized);
        if (status == ROCK_METADATA_OK) status = db_frame(&b, normalized.bytes, normalized.size);
        if (evaluated) for (uint64_t j = 0; j < params; ++j) free((void *)evaluated[j].bytes);
        free(original); free(evaluated); free((void *)evaluated_ret.bytes);
        free(declared.bytes); free(normalized.bytes);
    }
    free(ids); free((void *)subject.bytes);
    if (left && status == ROCK_METADATA_OK) status = ROCK_METADATA_INVALID;
    if (status == ROCK_METADATA_OK) { *output = b.bytes; *size = b.size; *member_count = members; }
    else free(b.bytes);
    return status;
}
typedef struct {
    uint64_t types, children, members;
    KeyView result;
    const unsigned char *bounds;
    uint64_t bounds_size;
} DictionaryTemplateView;
static int dictionary_template(KeyView input, DictionaryTemplateView *view) {
    const unsigned char *p = input.bytes; uint64_t left = input.size; KeyView definition, child;
    if (!p || left < 4 || left > KEY_LIMIT || memcmp(p, "DQ01", 4)) return 0;
    p += 4; left -= 4;
    if (!take_frame(&p, &left, &definition.bytes, &definition.size) || !valid_definition(definition.bytes, definition.size) ||
        !take_word(&p, &left, &view->types) || view->types > KEY_LIMIT / 8 ||
        !take_frame(&p, &left, &view->result.bytes, &view->result.size) ||
        !take_word(&p, &left, &view->children) || view->children > left / 8) return 0;
    view->bounds = p; view->bounds_size = left;
    for (uint64_t i = 0; i < view->children; ++i) if (!take_frame(&p, &left, &child.bytes, &child.size)) return 0;
    if (!take_word(&p, &left, &view->members) || view->members > left / 8) return 0;
    for (uint64_t i = 0; i < view->members; ++i)
        if (!take_frame(&p, &left, &definition.bytes, &definition.size) || !valid_definition(definition.bytes, definition.size)) return 0;
    return !left;
}
static RockMetadataStatus dictionary_bind(RockDictionary *d, KeyView template_key) {
    DictionaryTemplateView view;
    if (!dictionary_template(template_key, &view) || view.types != d->type_count || view.children != d->child_count || view.members != d->member_count) return ROCK_METADATA_INVALID;
    KeyView *types = calloc((size_t)(view.types ? view.types : 1), sizeof(*types));
    if (!types) return ROCK_METADATA_NOMEM;
    for (uint64_t i = 0; i < view.types; ++i) types[i] = (KeyView){d->types[i]->key, d->types[i]->key_size};
    unsigned char *schema = NULL; uint64_t n = 0, members = 0;
    RockMetadataStatus status = dictionary_schema(view.result, 0, view.types, types, &schema, &n, &members);
    if (status == ROCK_METADATA_OK && members != view.members) status = ROCK_METADATA_INVALID;
    const unsigned char *p = view.bounds; uint64_t left = view.bounds_size;
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < view.children; ++i) {
        KeyView bound; unsigned char *expected = NULL; uint64_t size = 0, count = 0;
        if (!take_frame(&p, &left, &bound.bytes, &bound.size)) { status = ROCK_METADATA_INVALID; break; }
        status = dictionary_schema(bound, 0, view.types, types, &expected, &size, &count);
        if (status == ROCK_METADATA_OK && !equal_bytes(size, expected, d->children[i]->key_size, d->children[i]->key)) status = ROCK_METADATA_INVALID;
        free(expected);
    }
    DictionaryBuffer b = {NULL, 0};
    if (status == ROCK_METADATA_OK) status = db_append(&b, (const unsigned char *)"DI01", 4);
    if (status == ROCK_METADATA_OK) status = db_frame(&b, schema, n);
    if (status == ROCK_METADATA_OK) status = db_frame(&b, template_key.bytes, template_key.size);
    if (status == ROCK_METADATA_OK) status = db_word(&b, view.types);
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < view.types; ++i) {
        status = db_frame(&b, d->types[i]->key, d->types[i]->key_size);
        if (status == ROCK_METADATA_OK) status = db_frame(&b, d->types[i]->drop_key, d->types[i]->drop_key_size);
    }
    if (status == ROCK_METADATA_OK) status = db_word(&b, view.children);
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < view.children; ++i) {
        status = db_frame(&b, d->children[i]->key, d->children[i]->key_size);
        if (status == ROCK_METADATA_OK) status = db_frame(&b, d->children[i]->binding, d->children[i]->binding_size);
    }
    free(types);
    if (status == ROCK_METADATA_OK) { d->key = schema; d->key_size = n; d->binding = b.bytes; d->binding_size = b.size; }
    else { free(schema); free(b.bytes); }
    return status;
}
static void dictionary_release(RockDictionary *d) {
    free((void *)d->key); free((void *)d->binding); free((void *)d->entries);
    free((void *)d->types); free((void *)d->children); free((void *)d->target);
}
static RockMetadataStatus dictionary_publish(RockDictionary *d, const RockDictionary **result) {
    uint64_t hash = key_hash(d->binding, d->binding_size);
    for (DictionaryRecord *r = dictionary_records; r; r = r->next) {
        if (r->namespace_kind != 3 || r->hash != hash || !equal_bytes(r->dictionary.binding_size, r->dictionary.binding, d->binding_size, d->binding)) continue;
        RockMetadataStatus status = r->state != ROCK_METADATA_READY ? ROCK_METADATA_CYCLE :
            r->dictionary.kind != d->kind || r->dictionary.member_count != d->member_count ||
            !equal_bytes(r->dictionary.target_size, r->dictionary.target, d->target_size, d->target) ? ROCK_METADATA_CONFLICT : ROCK_METADATA_OK;
        if (status == ROCK_METADATA_OK) *result = &r->dictionary;
        dictionary_release(d); return status;
    }
    DictionaryRecord *r = calloc(1, sizeof(*r));
    if (!r) { dictionary_release(d); return ROCK_METADATA_NOMEM; }
    r->namespace_kind = 3; r->hash = hash; r->state = ROCK_METADATA_BUILDING; r->dictionary = *d;
    r->dictionary.state = ROCK_METADATA_BUILDING; r->next = dictionary_records; dictionary_records = r;
    r->state = r->dictionary.state = ROCK_METADATA_READY; *result = &r->dictionary; return ROCK_METADATA_OK;
}
static RockMetadataStatus dictionary_arrays(const RockDescriptorTarget *target, RockDictionary *d, const RockDictionaryEntry *entries) {
    if (d->member_count > KEY_LIMIT / 8 || d->type_count > KEY_LIMIT / 8 || d->child_count > KEY_LIMIT / 8 ||
        (d->member_count && !entries)) return ROCK_METADATA_INVALID;
    d->entries = calloc((size_t)(d->member_count ? d->member_count : 1), sizeof(*d->entries));
    d->types = calloc((size_t)(d->type_count ? d->type_count : 1), sizeof(*d->types));
    d->children = calloc((size_t)(d->child_count ? d->child_count : 1), sizeof(*d->children));
    d->target = copy_bytes(target->key, target->key_size); d->target_size = target->key_size;
    d->abi = ROCK_DICTIONARY_ABI; d->state = ROCK_METADATA_READY; d->index_limit = target->index_limit;
    if (!d->entries || !d->types || !d->children || !d->target) return ROCK_METADATA_NOMEM;
    for (uint64_t i = 0; i < d->member_count; ++i) {
        if (!entries[i]) return ROCK_METADATA_INVALID;
        ((RockDictionaryEntry *)d->entries)[i] = entries[i];
    }
    return ROCK_METADATA_OK;
}
static RockMetadataStatus dictionary_freeze(const RockDescriptorTarget *target, const RockDictionary *input,
    const RockDictionary **active, unsigned depth, const RockDictionary **result) {
    if (!input || depth >= DEPTH_LIMIT) return ROCK_METADATA_CYCLE;
    for (unsigned i = 0; i < depth; ++i) if (active[i] == input) return ROCK_METADATA_CYCLE;
    if (input->state == ROCK_METADATA_BUILDING) return ROCK_METADATA_CYCLE;
    if (input->abi != ROCK_DICTIONARY_ABI || input->state != ROCK_METADATA_READY || (input->kind != 1 && input->kind != 2) ||
        input->index_limit != target->index_limit || !equal_bytes(input->target_size, input->target, target->key_size, target->key) ||
        !input->binding || input->binding_size < 4 || input->binding_size > KEY_LIMIT || memcmp(input->binding, "DI01", 4) ||
        (input->type_count && !input->types) || (input->child_count && !input->children)) return ROCK_METADATA_INVALID;
    const unsigned char *p = input->binding + 4; uint64_t left = input->binding_size - 4; KeyView schema, template_key;
    if (!take_frame(&p, &left, &schema.bytes, &schema.size) || !take_frame(&p, &left, &template_key.bytes, &template_key.size)) return ROCK_METADATA_INVALID;
    DictionaryTemplateView view;
    if (!dictionary_template(template_key, &view) || view.types != input->type_count || view.children != input->child_count || view.members != input->member_count ||
        (input->kind == 1 && (view.types || view.children))) return ROCK_METADATA_INVALID;
    RockDictionary d = {0}; d.kind = input->kind; d.member_count = view.members; d.type_count = view.types; d.child_count = view.children;
    RockMetadataStatus status = dictionary_arrays(target, &d, input->entries);
    active[depth] = input; const RockDescriptor *descriptor_active[DEPTH_LIMIT];
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < d.type_count; ++i)
        status = freeze(target, input->types[i], descriptor_active, 0, &((const RockDescriptor **)d.types)[i]);
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < d.child_count; ++i)
        status = dictionary_freeze(target, input->children[i], active, depth + 1, &((const RockDictionary **)d.children)[i]);
    if (status == ROCK_METADATA_OK) status = dictionary_bind(&d, template_key);
    if (status == ROCK_METADATA_OK && (!equal_bytes(d.key_size, d.key, input->key_size, input->key) ||
        !equal_bytes(d.key_size, d.key, schema.size, schema.bytes) || !equal_bytes(d.binding_size, d.binding, input->binding_size, input->binding))) status = ROCK_METADATA_INVALID;
    if (status == ROCK_METADATA_OK) return dictionary_publish(&d, result);
    dictionary_release(&d); return status;
}
RockMetadataStatus rock_dictionary_selected(const RockDescriptorTarget *target, const RockDictionaryTemplate *template,
    const RockDescriptor *const *types, const RockDictionary *const *children, const RockDictionary **result) {
    if (!result) return ROCK_METADATA_INVALID;
    *result = NULL;
    RockMetadataStatus status = target_valid(target);
    if (status != ROCK_METADATA_OK) return status;
    DictionaryTemplateView view;
    if (!template || template->abi != ROCK_DICTIONARY_ABI || !dictionary_template((KeyView){template->key, template->key_size}, &view) ||
        view.members != template->member_count || (view.types && !types) || (view.children && !children)) return ROCK_METADATA_INVALID;
    RockDictionary d = {0}; d.kind = 2; d.member_count = view.members; d.type_count = view.types; d.child_count = view.children;
    status = dictionary_arrays(target, &d, template->entries);
    if (status != ROCK_METADATA_OK) { dictionary_release(&d); return status; }
    if (pthread_mutex_lock(&metadata_mutex)) { dictionary_release(&d); return ROCK_METADATA_INVALID; }
    const RockDescriptor *descriptor_active[DEPTH_LIMIT]; const RockDictionary *active[DEPTH_LIMIT];
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < d.type_count; ++i)
        status = freeze(target, types[i], descriptor_active, 0, &((const RockDescriptor **)d.types)[i]);
    for (uint64_t i = 0; status == ROCK_METADATA_OK && i < d.child_count; ++i)
        status = dictionary_freeze(target, children[i], active, 0, &((const RockDictionary **)d.children)[i]);
    if (status == ROCK_METADATA_OK) status = dictionary_bind(&d, (KeyView){template->key, template->key_size});
    if (status == ROCK_METADATA_OK) status = dictionary_publish(&d, result);
    else dictionary_release(&d);
    pthread_mutex_unlock(&metadata_mutex); return status;
}
