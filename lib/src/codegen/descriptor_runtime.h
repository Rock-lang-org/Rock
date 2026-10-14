#ifndef ROCK_DESCRIPTOR_RUNTIME_H
#define ROCK_DESCRIPTOR_RUNTIME_H
#include <stddef.h>
#include <stdint.h>

#define ROCK_DESCRIPTOR_ABI 2u
#define ROCK_DESCRIPTOR_SHAPE 0u
#define ROCK_DESCRIPTOR_NATIVE 1u
#define ROCK_DESCRIPTOR_TUPLE 2u
#define ROCK_METADATA_BUILDING 1u
#define ROCK_METADATA_READY 2u
#define ROCK_METADATA_FAILED 3u

typedef struct RockDescriptor RockDescriptor;
typedef void (*RockDrop)(void *, const RockDescriptor *);
/* LLVM prefix and suffix are deliberately all native 64-bit slots. */
struct RockDescriptor {
    uint64_t size, alignment;
    RockDrop drop;
    uint64_t field_count;
    const uint64_t *offsets;
    uint64_t abi, kind, state;
    uint64_t key_size;
    const unsigned char *key;
    uint64_t drop_key_size;
    const unsigned char *drop_key;
    uint64_t child_count;
    const RockDescriptor *const *children;
    uint64_t target_key_size;
    const unsigned char *target_key;
    uint64_t index_limit;
    RockDrop user_drop;
    uint64_t argument_count;
    const RockDescriptor *const *arguments;
};

typedef struct {
    uint64_t abi, pointer_bits, little_endian, index_limit;
    uint64_t key_size;
    const unsigned char *key;
} RockDescriptorTarget;

typedef struct {
    uint64_t abi, key_size;
    const unsigned char *key; /* complete immutable RN01 selected template */
    uint64_t drop_key_size;
    const unsigned char *drop_key;
    RockDrop user_drop;
} RockNominalTemplate;

typedef enum {
    ROCK_METADATA_OK = 0, ROCK_METADATA_INVALID = 1,
    ROCK_METADATA_OVERFLOW = 2, ROCK_METADATA_CONFLICT = 3,
    ROCK_METADATA_CYCLE = 4, ROCK_METADATA_NOMEM = 5,
    ROCK_METADATA_TARGET = 6
} RockMetadataStatus;

#define ROCK_DICTIONARY_ABI 1u
typedef struct RockDictionary RockDictionary;
typedef void (*RockDictionaryEntry)(void *, const RockDictionary *, void *);
struct RockDictionary {
    uint64_t abi, kind, state, key_size;
    const unsigned char *key;
    uint64_t binding_size;
    const unsigned char *binding;
    uint64_t member_count;
    const RockDictionaryEntry *entries;
    uint64_t type_count;
    const RockDescriptor *const *types;
    uint64_t child_count;
    const RockDictionary *const *children;
    uint64_t target_size;
    const unsigned char *target;
    uint64_t index_limit;
};
typedef struct {
    uint64_t abi, key_size;
    const unsigned char *key;
    uint64_t member_count;
    const RockDictionaryEntry *entries;
} RockDictionaryTemplate;
RockMetadataStatus rock_dictionary_selected(const RockDescriptorTarget *,
    const RockDictionaryTemplate *, const RockDescriptor *const *types,
    const RockDictionary *const *children, const RockDictionary **result);

/* Inputs must be readable compiler-owned metadata, not untrusted addresses.
 * Native globals are unchanged; snapshots and all evidence arrays are frozen
 * into the shared process-lifetime registry. No payload/borrow is retained. */
RockMetadataStatus rock_descriptor_tuple(const RockDescriptorTarget *,
    uint64_t count, const RockDescriptor *const *children,
    const RockDescriptor **result);
void rock_descriptor_tuple_drop(void *, const RockDescriptor *);
RockMetadataStatus rock_descriptor_nominal(const RockDescriptorTarget *,
    const RockNominalTemplate *, const RockDescriptor *const *arguments,
    const RockDescriptor *const *fields, const RockDescriptor **result);
#endif
