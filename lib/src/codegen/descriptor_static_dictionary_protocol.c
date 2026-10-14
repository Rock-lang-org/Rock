#include "rock_descriptor_runtime.h"
#include "rock_dictionary_test_keys.h"
#include <assert.h>
#include <limits.h>
#include <pthread.h>
#include <stdatomic.h>
#include <string.h>

_Static_assert(sizeof(RockDictionary) == 128, "dictionary header ABI1");
_Static_assert(sizeof(RockDescriptor) == 160, "descriptor header ABI2");
static const RockDescriptorTarget target = {ROCK_DESCRIPTOR_ABI, 64, 1, INT64_MAX, sizeof(target_key), target_key};
static atomic_uint calls;
static void no_drop(void *out, const RockDescriptor *descriptor) { (void)out; (void)descriptor; }
/* Three declared logical arguments; the last is seed storage, never Self. */
static void make_entry(void *out, const RockDictionary *environment, void *seed) {
    assert(environment && environment->state == ROCK_METADATA_READY);
    atomic_fetch_add(&calls, 1);
    *(int64_t *)out = *(const int64_t *)seed + 1;
}
static void selected_entry(void *out, const RockDictionary *environment, void *seed) {
    assert(environment->type_count == 1 && environment->child_count == 1);
    const RockDictionary *child = environment->children[0];
    child->entries[0](out, child, seed);
}
static RockDictionaryEntry native_entries[] = {make_entry};
static const RockDictionaryEntry selected_entries[] = {selected_entry};
static const RockDictionaryTemplate selected = {1, sizeof(template_key), template_key, 1, selected_entries};
#define DESCRIPTOR(label) { .size = 8, .alignment = 8, .drop = no_drop, .abi = ROCK_DESCRIPTOR_ABI, .kind = 1, .state = 2, \
    .key_size = sizeof(type_##label##_key), .key = type_##label##_key, \
    .drop_key_size = sizeof(type_##label##_drop), .drop_key = type_##label##_drop, \
    .target_key_size = sizeof(target_key), .target_key = target_key, .index_limit = INT64_MAX }
#define DICTIONARY(label) { .abi = 1, .kind = 1, .state = 2, .key_size = sizeof(read_##label##_key), .key = read_##label##_key, \
    .binding_size = sizeof(read_##label##_binding), .binding = read_##label##_binding, .member_count = 1, .entries = native_entries, \
    .target_size = sizeof(target_key), .target = target_key, .index_limit = INT64_MAX }
static RockDescriptor type_a = DESCRIPTOR(a);
/* type_b is the actual producer Service nominal with one I64 field. */
static const uint64_t type_b_offsets[] = {0};
static const RockDescriptor *type_b_children[] = {&type_a};
static RockDescriptor type_b = DESCRIPTOR(b);
static RockDictionary child_a = DICTIONARY(a), child_b = DICTIONARY(b);
static void rejects(const unsigned char *key, uint64_t size) {
    RockDictionaryTemplate bad = selected; bad.key = key; bad.key_size = size;
    const RockDescriptor *type = &type_a; const RockDictionary *child = &child_a, *result = &child_a;
    assert(rock_dictionary_selected(&target, &bad, &type, &child, &result) == ROCK_METADATA_INVALID && !result);
}
static void *worker(void *variant) {
    const RockDescriptor *type = variant ? &type_b : &type_a;
    const RockDictionary *child = variant ? &child_b : &child_a, *last = NULL;
    for (unsigned i = 0; i < 100; ++i) {
        const RockDictionary *result = NULL;
        assert(rock_dictionary_selected(&target, &selected, &type, &child, &result) == ROCK_METADATA_OK);
        assert(!last || last == result); last = result;
        int64_t seed = 41, value = 0; result->entries[0](&value, result, &seed); assert(value == 42);
    }
    return (void *)last;
}
int main(void) {
    assert(type_a.key[4] == 'a' && type_b.key[4] == 'S');
    type_b.field_count = type_b.child_count = 1;
    type_b.offsets = type_b_offsets; type_b.children = type_b_children;
    const RockDescriptor *types[] = {&type_a}; const RockDictionary *children[] = {&child_a};
    const RockDictionary *a = NULL, *b = NULL, *again = NULL;
    assert(rock_dictionary_selected(&target, &selected, types, children, &a) == ROCK_METADATA_OK);
    assert(a->key_size == sizeof(relay_a_key) && !memcmp(a->key, relay_a_key, sizeof(relay_a_key)));
    assert(a->binding_size == sizeof(relay_a_binding) && !memcmp(a->binding, relay_a_binding, sizeof(relay_a_binding)));
    types[0] = NULL; children[0] = NULL; native_entries[0] = NULL;
    int64_t seed = 41, value = 0; a->entries[0](&value, a, &seed); assert(value == 42);
    assert(a->types[0] != &type_a && a->children[0] != &child_a && a->children[0]->entries[0] == make_entry);
    native_entries[0] = make_entry; types[0] = &type_a; children[0] = &child_a;
    assert(rock_dictionary_selected(&target, &selected, types, children, &again) == ROCK_METADATA_OK && again == a);
    types[0] = &type_b; children[0] = &child_b;
    assert(rock_dictionary_selected(&target, &selected, types, children, &b) == ROCK_METADATA_OK && b != a);
    const RockDescriptor *type = &type_a; const RockDictionary *wrong = &child_b;
    again = a; assert(rock_dictionary_selected(&target, &selected, &type, &wrong, &again) == ROCK_METADATA_INVALID && !again);
    RockDescriptor bad_type = type_a; bad_type.kind = 4; type = &bad_type;
    const RockDictionary *child = &child_a;
    again = a; assert(rock_dictionary_selected(&target, &selected, &type, &child, &again) == ROCK_METADATA_INVALID && !again);
#define REJECT(label) rejects(label, sizeof(label))
    REJECT(bad_ordinal); REJECT(bad_schema_abi); REJECT(bad_retention); REJECT(bad_returns); REJECT(bad_origin);
    REJECT(bad_shared); REJECT(bad_move); REJECT(bad_receiver_kind); REJECT(bad_static_local); REJECT(bad_static_arity);
    REJECT(bad_static_result);
    pthread_t threads[8];
    for (unsigned i = 0; i < 8; ++i) assert(!pthread_create(&threads[i], NULL, worker, i & 1 ? &type_b : NULL));
    for (unsigned i = 0; i < 8; ++i) { void *result; assert(!pthread_join(threads[i], &result)); assert(result == (i & 1 ? b : a)); }
    assert(atomic_load(&calls) == 801);
    return 0;
}
