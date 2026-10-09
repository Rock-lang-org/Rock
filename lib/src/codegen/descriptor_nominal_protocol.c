#include "rock_descriptor_runtime.h"
#include "rock_nominal_test_keys.h"
#include <assert.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(RockNominalTemplate) == 24, "selected template ABI");
static unsigned drops, order;
static void later_drop(void *p, const RockDescriptor *d) { (void)d; ++drops; order = order * 10 + (unsigned)*(uint64_t *)p; }
static void noop(void *p, const RockDescriptor *d) { (void)p; (void)d; }
static RockDescriptorTarget target = {1, 64, 1, INT64_MAX, sizeof(target_key), target_key};
static RockDescriptor later = {
    .size = 8, .alignment = 8, .drop = later_drop, .abi = 1, .kind = 1, .state = 2,
    .key_size = sizeof(later_key), .key = later_key, .drop_key_size = sizeof(later_drop_key), .drop_key = later_drop_key,
    .target_key_size = sizeof(target_key), .target_key = target_key, .index_limit = INT64_MAX,
};
static RockDescriptor tag = {
    .size = 8, .alignment = 8, .drop = noop, .abi = 1, .kind = 1, .state = 2,
    .key_size = sizeof(tag_key), .key = tag_key, .drop_key_size = sizeof(tag_drop_key), .drop_key = tag_drop_key,
    .target_key_size = sizeof(target_key), .target_key = target_key, .index_limit = INT64_MAX,
};
static RockNominalTemplate template = {1, sizeof(template_key), template_key};
static const RockDescriptor *arguments[1] = {&later};
static const RockDescriptor *fields[3];
static const RockDescriptor *record;
static void *concurrent(void *unused) {
    (void)unused; const RockDescriptor *result = NULL;
    assert(rock_descriptor_nominal(&target, &template, arguments, fields, &result) == ROCK_METADATA_OK);
    assert(result == record); return NULL;
}
int main(void) {
    if (NESTED) {
        const RockDescriptor *pair_inputs[2] = {&later, &later}; const RockDescriptor *pair = NULL;
        assert(rock_descriptor_tuple(&target, 2, pair_inputs, &pair) == ROCK_METADATA_OK);
        fields[0] = pair; fields[1] = &tag;
    } else { fields[0] = &later; fields[1] = &later; fields[2] = &tag; }
    assert(rock_descriptor_nominal(&target, &template, arguments, fields, &record) == ROCK_METADATA_OK);
    assert(record && record->size == 24 && record->alignment == 8 && record->kind == 1);
    assert(record->key_size == sizeof(product_key) && !memcmp(record->key, product_key, sizeof(product_key)));
    assert(record->drop_key_size == sizeof(product_drop_key) && !memcmp(record->drop_key, product_drop_key, sizeof(product_drop_key)));
    RockDescriptor native_product = *record;
    native_product.key = product_key; native_product.drop_key = product_drop_key;
    native_product.children = fields; native_product.drop = noop;
    const RockDescriptor *native_input[1] = {&native_product};
    const RockDescriptor *native_container = NULL;
    assert(rock_descriptor_tuple(&target, 1, native_input, &native_container) == ROCK_METADATA_OK);
    assert(native_container->children[0] == record);
    uint64_t payload[3] = {1, 2, 42}; record->drop(payload, record);
    assert(drops == 2 && order == 21);
    const RockDescriptor *result = NULL;
    unsigned char *copied = malloc(sizeof(template_key)); assert(copied);
    memcpy(copied, template_key, sizeof(template_key));
    RockNominalTemplate stack_template = {1, sizeof(template_key), copied};
    assert(rock_descriptor_nominal(&target, &stack_template, arguments, fields, &result) == ROCK_METADATA_OK && result == record);
    memset(copied, 0, sizeof(template_key)); free(copied);
    payload[0] = 1; payload[1] = 2; record->drop(payload, record); assert(drops == 4 && order == 2121);
    RockNominalTemplate bad = template; bad.abi = 99;
    assert(rock_descriptor_nominal(&target, &bad, arguments, fields, &result) != ROCK_METADATA_OK && !result);
    bad = (RockNominalTemplate){1, sizeof(bad_ordinal), bad_ordinal};
    assert(rock_descriptor_nominal(&target, &bad, arguments, fields, &result) != ROCK_METADATA_OK);
    bad = (RockNominalTemplate){1, sizeof(duplicate_fields), duplicate_fields};
    assert(rock_descriptor_nominal(&target, &bad, arguments, fields, &result) != ROCK_METADATA_OK);
    bad = (RockNominalTemplate){1, sizeof(conflicting_template), conflicting_template};
    assert(rock_descriptor_nominal(&target, &bad, arguments, fields, &result) == ROCK_METADATA_CONFLICT);
    const RockDescriptor *wrong[3] = {&tag, &tag, &tag};
    assert(rock_descriptor_nominal(&target, &template, arguments, wrong, &result) != ROCK_METADATA_OK);
    RockDescriptor shape = later; shape.kind = 0; const RockDescriptor *shape_argument[1] = {&shape};
    assert(rock_descriptor_nominal(&target, &template, shape_argument, fields, &result) != ROCK_METADATA_OK);
    shape = later; shape.state = 1;
    assert(rock_descriptor_nominal(&target, &template, shape_argument, fields, &result) == ROCK_METADATA_CYCLE);
    shape = later; shape.state = 3;
    assert(rock_descriptor_nominal(&target, &template, shape_argument, fields, &result) != ROCK_METADATA_OK);
    shape = later; shape.size = INT64_MAX;
    assert(rock_descriptor_nominal(&target, &template, shape_argument, fields, &result) == ROCK_METADATA_OVERFLOW);
    shape = later; shape.alignment = 3;
    assert(rock_descriptor_nominal(&target, &template, shape_argument, fields, &result) == ROCK_METADATA_INVALID);
    RockDescriptorTarget wrong_target = target; wrong_target.pointer_bits = 32;
    assert(rock_descriptor_nominal(&wrong_target, &template, arguments, fields, &result) == ROCK_METADATA_TARGET);
    RockDescriptor corrupt = *record; uint64_t wrong_offsets[3] = {8, 0, 16}; corrupt.offsets = wrong_offsets;
    const RockDescriptor *corrupt_input[1] = {&corrupt};
    assert(rock_descriptor_tuple(&target, 1, corrupt_input, &result) != ROCK_METADATA_OK);
    RockNominalTemplate empty_template = {1, sizeof(empty_template_key), empty_template_key};
    const RockDescriptor *empty = NULL;
    assert(rock_descriptor_nominal(&target, &empty_template, arguments, NULL, &empty) == ROCK_METADATA_OK);
    assert(empty->size == 0 && empty->alignment == 1 && empty->child_count == 0);
    assert(empty->key_size == sizeof(empty_product_key) && !memcmp(empty->key, empty_product_key, sizeof(empty_product_key)));
    assert(empty->drop_key_size == sizeof(empty_drop_key) && !memcmp(empty->drop_key, empty_drop_key, sizeof(empty_drop_key)));
    RockDescriptor native_empty = *empty;
    native_empty.key = empty_product_key; native_empty.drop_key = empty_drop_key; native_empty.drop = noop;
    const RockDescriptor *empty_input[1] = {&native_empty};
    assert(rock_descriptor_tuple(&target, 1, empty_input, &result) == ROCK_METADATA_OK && result->children[0] == empty);
    pthread_t threads[8];
    for (unsigned i = 0; i < 8; ++i) assert(!pthread_create(&threads[i], NULL, concurrent, NULL));
    for (unsigned i = 0; i < 8; ++i) assert(!pthread_join(threads[i], NULL));
    return 0;
}
