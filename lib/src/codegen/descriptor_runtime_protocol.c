/* Coordinator-executed native runtime protocol harness, not a Rock pipeline
 * acceptance fixture. Identity/template arrays are emitted by the Rust codec. */
#include "rock_descriptor_runtime.h"
#include "rock_descriptor_test_keys.h"
#include <assert.h>
#include <limits.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

static uint64_t order[16], dropped;
static void counted(void *value, const RockDescriptor *descriptor) {
    (void)descriptor; order[dropped++] = *(uint64_t *)value;
}
static void noop(void *value, const RockDescriptor *descriptor) { (void)value; (void)descriptor; }
#define NATIVE(name, n, a, destructor) { .size = n, .alignment = a, .drop = destructor, \
    .abi = ROCK_DESCRIPTOR_ABI, .kind = ROCK_DESCRIPTOR_NATIVE, .state = ROCK_METADATA_READY, \
    .key = name##_key, .key_size = sizeof(name##_key), \
    .drop_key = name##_drop, .drop_key_size = sizeof(name##_drop), \
    .target_key = target_key, .target_key_size = sizeof(target_key), .index_limit = INT64_MAX }
static RockDescriptor later = NATIVE(later, 8, 8, counted);
static RockDescriptor other = NATIVE(other, 8, 8, counted);
static RockDescriptor zero = NATIVE(zero, 0, 1, noop);
static RockDescriptor over = NATIVE(over, 64, 64, noop);
static RockDescriptor huge = NATIVE(huge, INT64_MAX, 1, noop);
static RockDescriptorTarget target = { ROCK_DESCRIPTOR_ABI, 64, 1, INT64_MAX, sizeof(target_key), target_key };

static const RockDescriptor *make_pair(const RockDescriptor *a, const RockDescriptor *b) {
    const RockDescriptor *inputs[] = {a, b}, *result = NULL;
    assert(rock_descriptor_tuple(&target, 2, inputs, &result) == ROCK_METADATA_OK);
    assert(result && result->state == ROCK_METADATA_READY && result->child_count == 2);
    return result;
}
typedef struct { unsigned different; const RockDescriptor *result; } ThreadInput;
static void *request(void *arg) {
    ThreadInput *input = arg;
    for (unsigned i = 0; i < 50; ++i)
        input->result = make_pair(&other, input->different ? &later : &other);
    return NULL;
}
static void rejected(RockDescriptor *child, RockMetadataStatus expected) {
    const RockDescriptor *inputs[] = {child}, *result = (const RockDescriptor *)1;
    assert(rock_descriptor_tuple(&target, 1, inputs, &result) == expected);
    assert(result == NULL);
}
int main(void) {
    const RockDescriptor *pair = make_pair(&later, &later);
    assert(pair->size == 16 && pair->alignment == 8 && pair->offsets[0] == 0 && pair->offsets[1] == 8);
    assert(pair->key_size == sizeof(pair_key) && !memcmp(pair->key, pair_key, sizeof(pair_key)));
    assert(pair->drop_key_size == sizeof(pair_drop) && !memcmp(pair->drop_key, pair_drop, sizeof(pair_drop)));
    assert(make_pair(&later, &later) == pair);
    const RockDescriptor *different = make_pair(&later, &other);
    assert(different != pair); /* Full keys, including forced-hash-collision mode. */
    uint64_t payload[] = {11, 22};
    pair->drop(payload, pair);
    assert(dropped == 2 && order[0] == 22 && order[1] == 11);

    /* Arrays, descriptors and their key bytes supplied on the stack cannot be
     * retained as environment authority by the process-lifetime descriptor. */
    RockDescriptor transient = later;
    unsigned char transient_key[sizeof(later_key)], transient_drop[sizeof(later_drop)];
    memcpy(transient_key, later_key, sizeof(later_key)); memcpy(transient_drop, later_drop, sizeof(later_drop));
    transient.key = transient_key; transient.drop_key = transient_drop;
    const RockDescriptor *stack_children[] = {&transient, &other}, *copied = NULL;
    assert(rock_descriptor_tuple(&target, 2, stack_children, &copied) == ROCK_METADATA_OK);
    assert(copied == different && copied->children[0] != &transient);
    memset(transient_key, 0, sizeof(transient_key)); memset(transient_drop, 0, sizeof(transient_drop));
    stack_children[0] = NULL; transient.drop = NULL;
    assert(copied->children[0]->drop && !memcmp(copied->children[0]->key, later_key, sizeof(later_key)));

    const RockDescriptor *nested = make_pair(pair, &other);
    assert(nested->key_size == sizeof(nested_key) && !memcmp(nested->key, nested_key, sizeof(nested_key)));
    assert(nested->drop_key_size == sizeof(nested_drop) && !memcmp(nested->drop_key, nested_drop, sizeof(nested_drop)));
    uint64_t nested_payload[] = {33, 44, 55};
    dropped = 0; nested->drop(nested_payload, nested);
    assert(dropped == 3 && order[0] == 55 && order[1] == 44 && order[2] == 33);
    /* A concrete native tuple descriptor must intern as the same semantic and
     * drop-template object as the runtime-composed tuple, regardless of thunk. */
    RockDescriptor native_pair = *pair;
    const uint64_t native_offsets[] = {0, 8};
    const RockDescriptor *native_children[] = {&later, &later};
    native_pair.offsets = native_offsets; native_pair.children = native_children;
    const RockDescriptor *wrapped_native = make_pair(&native_pair, &other);
    assert(wrapped_native == nested);
    assert(wrapped_native->children[0] == pair);

    const RockDescriptor *empty = NULL;
    assert(rock_descriptor_tuple(&target, 0, NULL, &empty) == ROCK_METADATA_OK);
    assert(empty->size == 0 && empty->alignment == 1 && empty->child_count == 0);
    const RockDescriptor *aligned = make_pair(&zero, &over);
    assert(aligned->size == 64 && aligned->alignment == 64 && aligned->offsets[0] == 0 && aligned->offsets[1] == 0);
    const RockDescriptor *zeros = make_pair(&zero, &zero);
    assert(zeros->size == 0 && zeros->alignment == 1);

    RockDescriptor bad = later;
    bad.abi = 99; rejected(&bad, ROCK_METADATA_INVALID);
    bad = later; bad.state = ROCK_METADATA_BUILDING; rejected(&bad, ROCK_METADATA_CYCLE);
    bad = later; bad.state = ROCK_METADATA_FAILED; rejected(&bad, ROCK_METADATA_INVALID);
    bad = later; bad.kind = ROCK_DESCRIPTOR_SHAPE; rejected(&bad, ROCK_METADATA_INVALID);
    bad = later; bad.key_size--; rejected(&bad, ROCK_METADATA_INVALID);
    bad = later; bad.drop_key = other_drop; bad.drop_key_size = sizeof(other_drop); rejected(&bad, ROCK_METADATA_INVALID);
    bad = later; bad.alignment = 0; rejected(&bad, ROCK_METADATA_INVALID);
    bad = later; bad.alignment = 3; rejected(&bad, ROCK_METADATA_INVALID);
    bad = later; bad.size = INT64_MAX; rejected(&bad, ROCK_METADATA_OVERFLOW);
    bad = later; bad.size = 16; rejected(&bad, ROCK_METADATA_CONFLICT);
    bad = later; bad.drop_key = later_alt_drop; bad.drop_key_size = sizeof(later_alt_drop); rejected(&bad, ROCK_METADATA_CONFLICT);
    bad = later; bad.alignment = 16; rejected(&bad, ROCK_METADATA_CONFLICT);
    bad = later; bad.target_key_size--; rejected(&bad, ROCK_METADATA_INVALID);
    bad = native_pair;
    const RockDescriptor *cycle_children[] = {&bad, &later};
    bad.children = cycle_children; rejected(&bad, ROCK_METADATA_CYCLE);
    bad = native_pair; bad.field_count = 1; rejected(&bad, ROCK_METADATA_INVALID);
    bad = native_pair; bad.field_count = bad.child_count = 1; rejected(&bad, ROCK_METADATA_INVALID);
    const uint64_t bad_offsets[] = {0, 9};
    bad = native_pair; bad.offsets = bad_offsets; rejected(&bad, ROCK_METADATA_INVALID);
    bad = native_pair; bad.size = 24; rejected(&bad, ROCK_METADATA_CONFLICT);

    const RockDescriptor *overflow_inputs[] = {&huge, &later}, *result = NULL;
    assert(rock_descriptor_tuple(&target, 2, overflow_inputs, &result) == ROCK_METADATA_OVERFLOW && !result);
    assert(rock_descriptor_tuple(&target, UINT64_MAX, overflow_inputs, &result) == ROCK_METADATA_INVALID && !result);
    assert(rock_descriptor_tuple(&target, 1, NULL, &result) == ROCK_METADATA_INVALID && !result);
    RockDescriptorTarget unsupported = target; unsupported.pointer_bits = 32;
    assert(rock_descriptor_tuple(&unsupported, 0, NULL, &result) == ROCK_METADATA_TARGET && !result);
    unsupported = target; unsupported.little_endian = 0;
    assert(rock_descriptor_tuple(&unsupported, 0, NULL, &result) == ROCK_METADATA_TARGET && !result);
    unsupported = target; unsupported.key_size--;
    assert(rock_descriptor_tuple(&unsupported, 0, NULL, &result) == ROCK_METADATA_TARGET && !result);

    pthread_t threads[12]; ThreadInput inputs[12];
    for (unsigned i = 0; i < 12; ++i) {
        inputs[i] = (ThreadInput){i % 2, NULL};
        assert(!pthread_create(&threads[i], NULL, request, &inputs[i]));
    }
    for (unsigned i = 0; i < 12; ++i) {
        assert(!pthread_join(threads[i], NULL));
        assert(inputs[i].result == make_pair(&other, inputs[i].different ? &later : &other));
    }
    return 0;
}
