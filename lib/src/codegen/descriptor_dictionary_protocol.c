#include "rock_descriptor_runtime.h"
#include "rock_dictionary_test_keys.h"
#include <assert.h>
#include <limits.h>
#include <pthread.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>

static const RockDescriptorTarget target = {ROCK_DESCRIPTOR_ABI, 64, 1, INT64_MAX, sizeof(target_key), target_key};
static atomic_uint calls;
static void no_drop(void *payload, const RockDescriptor *descriptor) { (void)payload; (void)descriptor; }
static void read_entry(void *out, const RockDictionary *environment, void *receiver) {
    assert(environment && environment->state == ROCK_METADATA_READY);
    atomic_fetch_add(&calls, 1);
    *(int64_t *)out = *(const int64_t *)receiver + 1;
}
static void relay_entry(void *out, const RockDictionary *environment, void *receiver) {
    assert(environment->type_count == 1 && environment->child_count == 1);
    const RockDictionary *read = environment->children[0];
    read->entries[0](out, read, receiver);
}
static const RockDictionaryEntry selected_entries[] = {relay_entry};
static RockDictionaryEntry native_entries[] = {read_entry};
static const RockDictionaryTemplate selected = {1, sizeof(template_key), template_key, 1, selected_entries};
static RockDescriptor type_a = {
    .size = 8, .alignment = 8, .drop = no_drop, .abi = ROCK_DESCRIPTOR_ABI, .kind = 1, .state = 2,
    .key_size = sizeof(type_a_key), .key = type_a_key, .drop_key_size = sizeof(type_a_drop), .drop_key = type_a_drop,
    .target_key_size = sizeof(target_key), .target_key = target_key, .index_limit = INT64_MAX
};
static RockDescriptor type_b = {
    .size = 8, .alignment = 8, .drop = no_drop, .abi = ROCK_DESCRIPTOR_ABI, .kind = 1, .state = 2,
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
static void rejects(const RockDictionaryTemplate *template, const RockDescriptor *type, const RockDictionary *child, RockMetadataStatus status) {
    const RockDictionary *result = &read_a;
    assert(rock_dictionary_selected(&target, template, &type, &child, &result) == status);
    assert(result == NULL);
}
static void *worker(void *wide) {
    const RockDescriptor *type = wide ? &type_b : &type_a;
    const RockDictionary *child = wide ? &read_b : &read_a;
    const RockDictionary *last = NULL;
    for (unsigned i = 0; i < 100; ++i) {
        const RockDictionary *d = NULL;
        assert(rock_dictionary_selected(&target, &selected, &type, &child, &d) == ROCK_METADATA_OK);
        assert(!last || last == d); last = d;
        int64_t receiver = 41, result = 0; d->entries[0](&result, d, &receiver); assert(result == 42);
    }
    return (void *)last;
}
int main(void) {
    const RockDescriptor *types[] = {&type_a}; const RockDictionary *children[] = {&read_a};
    const RockDictionary *a = NULL, *b = NULL, *again = NULL;
    assert(rock_dictionary_selected(&target, &selected, types, children, &a) == ROCK_METADATA_OK);
    assert(a->key_size == sizeof(relay_a_key) && !memcmp(a->key, relay_a_key, sizeof(relay_a_key)));
    assert(a->binding_size == sizeof(relay_a_binding) && !memcmp(a->binding, relay_a_binding, sizeof(relay_a_binding)));
    types[0] = NULL; children[0] = NULL; native_entries[0] = NULL;
    int64_t receiver = 41, result = 0; a->entries[0](&result, a, &receiver); assert(result == 42);
    assert(a->types[0] != &type_a && a->children[0] != &read_a && a->children[0]->entries[0] == read_entry);
    native_entries[0] = read_entry;
    types[0] = &type_a; children[0] = &read_a;
    assert(rock_dictionary_selected(&target, &selected, types, children, &again) == ROCK_METADATA_OK && again == a);
    types[0] = &type_b; children[0] = &read_b;
    assert(rock_dictionary_selected(&target, &selected, types, children, &b) == ROCK_METADATA_OK && b != a);
    assert(a->types[0]->size == b->types[0]->size && (a->binding_size != b->binding_size || memcmp(a->binding, b->binding, a->binding_size)));
    rejects(&selected, &type_a, &read_b, ROCK_METADATA_INVALID);
    RockDictionary child = read_a;
    child.kind = 0; rejects(&selected, &type_a, &child, ROCK_METADATA_INVALID);
    child = read_a; child.abi = 9; rejects(&selected, &type_a, &child, ROCK_METADATA_INVALID);
    child = read_a; child.state = ROCK_METADATA_BUILDING; rejects(&selected, &type_a, &child, ROCK_METADATA_CYCLE);
    child = read_a; child.state = ROCK_METADATA_FAILED; rejects(&selected, &type_a, &child, ROCK_METADATA_INVALID);
    child = read_a; child.member_count = 0; rejects(&selected, &type_a, &child, ROCK_METADATA_INVALID);
    child = read_a; child.entries = NULL; rejects(&selected, &type_a, &child, ROCK_METADATA_INVALID);
    child = read_a; child.kind = 2; rejects(&selected, &type_a, &child, ROCK_METADATA_CONFLICT);
    child = read_a; child.key = read_b_key; child.key_size = sizeof(read_b_key); rejects(&selected, &type_a, &child, ROCK_METADATA_INVALID);
    child = read_a; child.binding = read_b_binding; child.binding_size = sizeof(read_b_binding); rejects(&selected, &type_a, &child, ROCK_METADATA_INVALID);
    child = read_a; child.target_size--; rejects(&selected, &type_a, &child, ROCK_METADATA_INVALID);
    child = read_a; child.index_limit--; rejects(&selected, &type_a, &child, ROCK_METADATA_INVALID);
    child = *a; const RockDictionary *cycle[] = {&child}; child.children = cycle;
    rejects(&selected, &type_a, &child, ROCK_METADATA_CYCLE);
    RockDescriptor type = type_a; type.kind = 0; rejects(&selected, &type, &read_a, ROCK_METADATA_INVALID);
    type = type_a; type.alignment = 3; rejects(&selected, &type, &read_a, ROCK_METADATA_INVALID);
    RockDictionaryTemplate malformed = selected; malformed.abi = 9; rejects(&malformed, &type_a, &read_a, ROCK_METADATA_INVALID);
    malformed = selected; malformed.member_count = 0; rejects(&malformed, &type_a, &read_a, ROCK_METADATA_INVALID);
    malformed = selected; malformed.entries = NULL; rejects(&malformed, &type_a, &read_a, ROCK_METADATA_INVALID);
    unsigned char *bytes = malloc(sizeof(template_key) + 1); assert(bytes);
    memcpy(bytes, template_key, sizeof(template_key)); bytes[sizeof(template_key)] = 0;
    malformed = selected; malformed.key = bytes; malformed.key_size++; rejects(&malformed, &type_a, &read_a, ROCK_METADATA_INVALID);
    memcpy(bytes, bad_ordinal, sizeof(bad_ordinal)); malformed.key_size = sizeof(bad_ordinal); rejects(&malformed, &type_a, &read_a, ROCK_METADATA_INVALID);
    memcpy(bytes, bad_schema_abi, sizeof(bad_schema_abi)); malformed.key_size = sizeof(bad_schema_abi); rejects(&malformed, &type_a, &read_a, ROCK_METADATA_INVALID);
    memcpy(bytes, bad_retention, sizeof(bad_retention)); malformed.key_size = sizeof(bad_retention); rejects(&malformed, &type_a, &read_a, ROCK_METADATA_INVALID);
    memcpy(bytes, bad_returns, sizeof(bad_returns)); malformed.key_size = sizeof(bad_returns); rejects(&malformed, &type_a, &read_a, ROCK_METADATA_INVALID);
    memcpy(bytes, bad_origin, sizeof(bad_origin)); malformed.key_size = sizeof(bad_origin); rejects(&malformed, &type_a, &read_a, ROCK_METADATA_INVALID);
    free(bytes);
    again = a; assert(rock_dictionary_selected(&target, &selected, NULL, children, &again) == ROCK_METADATA_INVALID && !again);
    again = a; assert(rock_dictionary_selected(&target, &selected, types, NULL, &again) == ROCK_METADATA_INVALID && !again);
    RockDescriptorTarget incompatible = target; incompatible.pointer_bits = 32;
    types[0] = &type_a; children[0] = &read_a; again = a;
    assert(rock_dictionary_selected(&incompatible, &selected, types, children, &again) == ROCK_METADATA_TARGET && !again);
    pthread_t threads[8];
    for (unsigned i = 0; i < 8; ++i) assert(!pthread_create(&threads[i], NULL, worker, i & 1 ? &type_b : NULL));
    for (unsigned i = 0; i < 8; ++i) { void *value; assert(!pthread_join(threads[i], &value)); assert(value == (i & 1 ? b : a)); }
    assert(atomic_load(&calls) == 801);
    return 0;
}
