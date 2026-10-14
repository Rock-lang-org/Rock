#include "rock_descriptor_runtime.h"
#include "rock_nominal_test_keys.h"
#include <assert.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

_Static_assert(sizeof(RockNominalTemplate) == 48, "selected template ABI");
static unsigned drops, order;
static void later_drop(void *p, const RockDescriptor *d) { (void)d; ++drops; order = order * 10 + (unsigned)*(uint64_t *)p; }
static void noop(void *p, const RockDescriptor *d) { (void)p; (void)d; }
static void phantom_drop(void *p, const RockDescriptor *d) {
    (void)p;
    assert(d->argument_count == 1 && d->arguments && d->arguments[0]->size == 8);
    assert(d->child_count == 0);
    ++drops;
}
static RockDescriptorTarget target = {ROCK_DESCRIPTOR_ABI, 64, 1, INT64_MAX, sizeof(target_key), target_key};
static RockDescriptor later = {
    .size = 8, .alignment = 8, .drop = later_drop, .abi = ROCK_DESCRIPTOR_ABI, .kind = 1, .state = 2,
    .key_size = sizeof(later_key), .key = later_key, .drop_key_size = sizeof(later_drop_key), .drop_key = later_drop_key,
    .target_key_size = sizeof(target_key), .target_key = target_key, .index_limit = INT64_MAX,
};
static RockDescriptor tag = {
    .size = 8, .alignment = 8, .drop = noop, .abi = ROCK_DESCRIPTOR_ABI, .kind = 1, .state = 2,
    .key_size = sizeof(tag_key), .key = tag_key, .drop_key_size = sizeof(tag_drop_key), .drop_key = tag_drop_key,
    .target_key_size = sizeof(target_key), .target_key = target_key, .index_limit = INT64_MAX,
};
static RockNominalTemplate template = {ROCK_DESCRIPTOR_ABI, sizeof(template_key), template_key, 0, NULL, NULL};
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
    RockNominalTemplate stack_template = {ROCK_DESCRIPTOR_ABI, sizeof(template_key), copied, 0, NULL, NULL};
    assert(rock_descriptor_nominal(&target, &stack_template, arguments, fields, &result) == ROCK_METADATA_OK && result == record);
    memset(copied, 0, sizeof(template_key)); free(copied);
    payload[0] = 1; payload[1] = 2; record->drop(payload, record); assert(drops == 4 && order == 2121);
    RockNominalTemplate bad = template; bad.abi = 99;
    assert(rock_descriptor_nominal(&target, &bad, arguments, fields, &result) != ROCK_METADATA_OK && !result);
    bad = (RockNominalTemplate){ROCK_DESCRIPTOR_ABI, sizeof(bad_ordinal), bad_ordinal, 0, NULL, NULL};
    assert(rock_descriptor_nominal(&target, &bad, arguments, fields, &result) != ROCK_METADATA_OK);
    bad = (RockNominalTemplate){ROCK_DESCRIPTOR_ABI, sizeof(duplicate_fields), duplicate_fields, 0, NULL, NULL};
    assert(rock_descriptor_nominal(&target, &bad, arguments, fields, &result) != ROCK_METADATA_OK);
    bad = (RockNominalTemplate){ROCK_DESCRIPTOR_ABI, sizeof(conflicting_template), conflicting_template, 0, NULL, NULL};
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
    RockNominalTemplate empty_template = {ROCK_DESCRIPTOR_ABI, sizeof(empty_template_key), empty_template_key, 0, NULL, NULL};
    RockNominalTemplate selected_template = {
        ROCK_DESCRIPTOR_ABI, sizeof(selected_template_key), selected_template_key,
        sizeof(selected_drop_key), selected_drop_key, phantom_drop,
    };
    const RockDescriptor *selected = NULL;
    assert(rock_descriptor_nominal(&target, &selected_template, arguments, NULL, &selected) == ROCK_METADATA_OK);
    assert(selected && selected->size == 0 && selected->alignment == 1 && selected->user_drop == phantom_drop);
    drops = 0; selected->drop(NULL, selected); assert(drops == 1);
    RockNominalTemplate invalid_selected = selected_template;
    invalid_selected.user_drop = NULL;
    assert(rock_descriptor_nominal(&target, &invalid_selected, arguments, NULL, &result) == ROCK_METADATA_INVALID && !result);
    unsigned char corrupt_drop[sizeof(selected_drop_key)];
    memcpy(corrupt_drop, selected_drop_key, sizeof(corrupt_drop));
    /* The single RF01 variable recipe is the final framed word. */
    corrupt_drop[sizeof(corrupt_drop) - 8] = 99;
    invalid_selected = selected_template; invalid_selected.drop_key = corrupt_drop;
    assert(rock_descriptor_nominal(&target, &invalid_selected, arguments, NULL, &result) == ROCK_METADATA_INVALID && !result);
    RockDescriptor forged_selected = *selected;
    const RockDescriptor *bad_args[] = {&tag};
    forged_selected.arguments = bad_args;
    const RockDescriptor *wrapper[] = {&forged_selected};
    assert(rock_descriptor_tuple(&target, 1, wrapper, &result) == ROCK_METADATA_INVALID && !result);
    forged_selected = *selected; forged_selected.argument_count = 0;
    assert(rock_descriptor_tuple(&target, 1, wrapper, &result) == ROCK_METADATA_INVALID && !result);
    unsigned char transient_drop[sizeof(selected_drop_key)];
    memcpy(transient_drop, selected_drop_key, sizeof(transient_drop));
    invalid_selected = selected_template; invalid_selected.drop_key = transient_drop;
    const RockDescriptor *copied_selected = NULL;
    assert(rock_descriptor_nominal(&target, &invalid_selected, arguments, NULL, &copied_selected) == ROCK_METADATA_OK && copied_selected == selected);
    memset(transient_drop, 0, sizeof(transient_drop));
    copied_selected->drop(NULL, copied_selected); assert(drops == 2);
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
