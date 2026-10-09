#include "rock_descriptor_runtime.h"
#include "rock_dictionary_test_keys.h"
#include <assert.h>
#include <limits.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>

static const RockDescriptorTarget target = {1, 64, 1, INT64_MAX, sizeof(target_key), target_key};
static atomic_uint calls, drops;
static void no_drop(void *payload, const RockDescriptor *descriptor) { (void)payload; (void)descriptor; }
static void counted_drop(void *payload, const RockDescriptor *descriptor) { (void)payload; (void)descriptor; atomic_fetch_add(&drops, 1); }
static void read_entry(void *out, const RockDictionary *environment, void *receiver) {
    assert(environment && environment->state == ROCK_METADATA_READY);
    atomic_fetch_add(&calls, 1); *(int64_t *)out = *(const int64_t *)receiver + 1;
}
typedef void (*ForwardEntry)(void *, const RockDictionary *, void *, const RockDescriptor *, void *);
/* DictionaryEntry is opaque storage to the runtime; only checked member ABI
 * consumers invoke it. Round-trip the native function representation here. */
typedef union { ForwardEntry forward; RockDictionaryEntry storage; } Entry;
static void forward_entry(void *out, const RockDictionary *environment, void *receiver, const RockDescriptor *u, void *value) {
    assert(environment->type_count == 1 && environment->child_count == 1);
    assert(u && u != environment->types[0]);
    const RockDictionary *read = environment->children[0]; int64_t result = 0;
    read->entries[0](&result, read, receiver); assert(result == 42);
    memcpy(out, value, (size_t)u->size);
}
static RockDictionaryEntry native_entries[] = {read_entry};
static RockDescriptor type_a = {
    .size = 8, .alignment = 8, .drop = no_drop, .abi = 1, .kind = 1, .state = 2,
    .key_size = sizeof(type_a_key), .key = type_a_key, .drop_key_size = sizeof(type_a_drop), .drop_key = type_a_drop,
    .target_key_size = sizeof(target_key), .target_key = target_key, .index_limit = INT64_MAX
};
static RockDescriptor type_b = {
    .size = 8, .alignment = 8, .drop = no_drop, .abi = 1, .kind = 1, .state = 2,
    .key_size = sizeof(type_b_key), .key = type_b_key, .drop_key_size = sizeof(type_b_drop), .drop_key = type_b_drop,
    .target_key_size = sizeof(target_key), .target_key = target_key, .index_limit = INT64_MAX
};
static RockDictionary read_a = {
    .abi = 1, .kind = 1, .state = 2, .key_size = sizeof(read_a_key), .key = read_a_key,
    .binding_size = sizeof(read_a_binding), .binding = read_a_binding, .member_count = 1, .entries = native_entries,
    .target_size = sizeof(target_key), .target = target_key, .index_limit = INT64_MAX
};
static RockDictionary read_b = {
    .abi = 1, .kind = 1, .state = 2, .key_size = sizeof(read_b_key), .key = read_b_key,
    .binding_size = sizeof(read_b_binding), .binding = read_b_binding, .member_count = 1, .entries = native_entries,
    .target_size = sizeof(target_key), .target = target_key, .index_limit = INT64_MAX
};
static RockDictionaryTemplate selected;
static void invoke(const RockDictionary *d, uint64_t words, int counted) {
    /* U is stack-lived per call. The environment must not retain it. */
    RockDescriptor u = {.size = words * 8, .alignment = 8, .drop = counted_drop};
    uint64_t value[3] = {42, 91, 137}, output[3] = {0, 0, 0}; int64_t maker = 41;
    Entry entry = {.storage = d->entries[0]}; entry.forward(output, d, &maker, &u, value);
    assert(!memcmp(output, value, (size_t)u.size));
    if (counted) u.drop(output, &u);
    assert(d->type_count == 1 && d->child_count == 1 && d->types[0] != &u);
}
static void *worker(void *wide) {
    const RockDescriptor *type = wide ? &type_b : &type_a; const RockDictionary *child = wide ? &read_b : &read_a;
    const RockDictionary *last = NULL;
    for (unsigned i = 0; i < 100; ++i) {
        const RockDictionary *d = NULL; assert(rock_dictionary_selected(&target, &selected, &type, &child, &d) == ROCK_METADATA_OK);
        assert(!last || last == d); last = d; invoke(d, wide ? 3 : 1, 0);
    }
    return (void *)last;
}
int main(void) {
    Entry entry = {.forward = forward_entry}; RockDictionaryEntry entries[] = {entry.storage};
    selected = (RockDictionaryTemplate){1, sizeof(template_key), template_key, 1, entries};
    const RockDescriptor *types[] = {&type_a}; const RockDictionary *children[] = {&read_a}; const RockDictionary *a = NULL, *b = NULL;
    assert(rock_dictionary_selected(&target, &selected, types, children, &a) == ROCK_METADATA_OK);
    assert(a->key_size == sizeof(relay_a_key) && !memcmp(a->key, relay_a_key, sizeof(relay_a_key)));
    assert(a->binding_size == sizeof(relay_a_binding) && !memcmp(a->binding, relay_a_binding, sizeof(relay_a_binding)));
    types[0] = NULL; children[0] = NULL; entries[0] = NULL; native_entries[0] = NULL;
    invoke(a, 1, 1); invoke(a, 3, 1); assert(atomic_load(&calls) == 2 && atomic_load(&drops) == 2);
    assert(a->types[0] != &type_a && a->children[0] != &read_a && a->children[0]->entries[0] == read_entry);
    entries[0] = entry.storage; native_entries[0] = read_entry;
    types[0] = &type_b; children[0] = &read_b;
    assert(rock_dictionary_selected(&target, &selected, types, children, &b) == ROCK_METADATA_OK && b != a);
    const unsigned char *bad[] = {bad_ordinal, bad_schema_abi, bad_retention, bad_returns, bad_origin, bad_local, bad_kind, bad_bounds};
    const uint64_t sizes[] = {sizeof(bad_ordinal), sizeof(bad_schema_abi), sizeof(bad_retention), sizeof(bad_returns), sizeof(bad_origin), sizeof(bad_local), sizeof(bad_kind), sizeof(bad_bounds)};
    for (unsigned i = 0; i < sizeof(bad) / sizeof(*bad); ++i) {
        RockDictionaryTemplate corrupt = selected; corrupt.key = bad[i]; corrupt.key_size = sizes[i];
        const RockDictionary *result = a; types[0] = &type_a; children[0] = &read_a;
        assert(rock_dictionary_selected(&target, &corrupt, types, children, &result) == ROCK_METADATA_INVALID && !result);
    }
    pthread_t threads[8];
    for (unsigned i = 0; i < 8; ++i) assert(!pthread_create(&threads[i], NULL, worker, i & 1 ? &type_b : NULL));
    for (unsigned i = 0; i < 8; ++i) { void *value; assert(!pthread_join(threads[i], &value)); assert(value == (i & 1 ? b : a)); }
    assert(atomic_load(&calls) == 802 && atomic_load(&drops) == 2);
    return 0;
}
