#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct RtBackend RtBackend;

typedef struct RtBuffer {
    uint8_t *data;
    size_t len;
} RtBuffer;

enum {
    RT_OK = 0,
    RT_ERR = 1,
    RT_EMPTY = 2
};

RtBackend *rt_backend_create(void);
void rt_backend_destroy(RtBackend *handle);
void rt_buffer_free(RtBuffer buffer);
int rt_poll_event(RtBackend *handle, RtBuffer *out);
int rt_job_cancel(RtBackend *handle, uint64_t job_id);
int rt_proof_release(RtBackend *handle, uint64_t proof_id);
int rt_version(RtBackend *handle, RtBuffer *out);
int rt_settings_load(RtBackend *handle, RtBuffer *out);
int rt_settings_save(RtBackend *handle, const uint8_t *json, size_t len, RtBuffer *out);
int rt_settings_path(RtBackend *handle, RtBuffer *out);
int rt_state_directory(RtBackend *handle, RtBuffer *out);
int rt_process_default_path(RtBackend *handle, RtBuffer *out);
int rt_process_discover(RtBackend *handle, RtBuffer *out);
int rt_process_launch(RtBackend *handle, const char *path, uint64_t *out_job);
int rt_process_stop(RtBackend *handle, uint64_t session_id, uint64_t *out_job);
int rt_hunt_find(RtBackend *handle, uint64_t session_id, uint64_t *out_job);
int rt_hunt_apply(RtBackend *handle, uint64_t proof_id, const uint8_t *json, size_t len, uint64_t *out_job);
int rt_update_check(RtBackend *handle, const char *current, uint64_t *out_job);
int rt_diagnostics_paths(RtBackend *handle, RtBuffer *out);
int rt_diagnostics_recent_lines(RtBackend *handle, RtBuffer *out);
int rt_stats_metadata(RtBackend *handle, int chinese, RtBuffer *out);

#ifdef __cplusplus
}
#endif
