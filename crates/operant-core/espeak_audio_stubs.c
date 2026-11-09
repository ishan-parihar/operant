// Stub implementations for espeak-ng audio backend symbols
// These are needed when espeak-rs-sys builds espeak-ng from source
// without the audio backend plugins (pulseaudio, portaudio, etc.)
// Only used for test compilation - TTS audio output won't work.

#include <stddef.h>

typedef struct { int dummy; } audio_object_t;

audio_object_t* audio_object_open(int device, int sample_rate, int channels, int latency) {
    (void)device; (void)sample_rate; (void)channels; (void)latency;
    return NULL;
}

int audio_object_write(audio_object_t* obj, const short* samples, int count) {
    (void)obj; (void)samples; (void)count;
    return -1;
}

const char* audio_object_strerror(int error) {
    (void)error;
    return "audio backend not available (stub)";
}

int audio_object_close(audio_object_t* obj) {
    (void)obj;
    return 0;
}

int audio_object_flush(audio_object_t* obj) {
    (void)obj;
    return 0;
}

int audio_object_drain(audio_object_t* obj) {
    (void)obj;
    return 0;
}

void* create_audio_device_object(int device, int sample_rate) {
    (void)device; (void)sample_rate;
    return NULL;
}

// ── libsonic (the audio backend's speed-up helper) ───────────────────────
//
// espeak-ng compiles its sonic speed-up path whenever its cmake build
// detects a system libsonic (USE_LIBSONIC is ON on any host that has
// libsonic installed — it is detected, not configured). The final link then
// needs -lsonic, and NOTHING emits it: espeak-rs-sys's build script forwards
// only the libraries it built itself (speechPlayer/espeak-ng/ucd), never the
// system dependency espeak-ng linked PRIVATE. On such a host the binary
// therefore fails to link with undefined sonic* symbols — operant-cli
// included (RT-416).
//
// Stubbing the speed-up is the same bargain as the audio-backend stubs
// above: the stream is NULL, so espeak skips the speed-up and falls back to
// its normal path — which is itself stubbed here, so no audio output either
// way. The types are local on purpose: the point is to need neither the
// header nor the library.

typedef struct sonic_stream_stub* sonic_stream_stub_t;

sonic_stream_stub_t sonicCreateStream(int sample_rate, int num_channels) {
    (void)sample_rate; (void)num_channels;
    return NULL;
}

void sonicDestroyStream(sonic_stream_stub_t stream) {
    (void)stream;
}

int sonicWriteShortToStream(sonic_stream_stub_t stream, short* samples, int num_samples) {
    (void)stream; (void)samples; (void)num_samples;
    return 0;
}

int sonicReadShortFromStream(sonic_stream_stub_t stream, short* samples, int max_samples) {
    (void)stream; (void)samples; (void)max_samples;
    return 0;
}

int sonicFlushStream(sonic_stream_stub_t stream) {
    (void)stream;
    return 0;
}

float sonicGetSpeed(sonic_stream_stub_t stream) {
    (void)stream;
    return 1.0f;
}

void sonicSetSpeed(sonic_stream_stub_t stream, float speed) {
    (void)stream; (void)speed;
}
