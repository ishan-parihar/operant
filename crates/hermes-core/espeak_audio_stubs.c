// Stub implementations for espeak-ng audio backend symbols
// These are needed when espeak-rs-sys builds espeak-ng from source
// without the audio backend plugins (pulseaudio, portaudio, etc.)
// Only used for test compilation - TTS audio output won't work.

#include <stddef.h>

typedef struct { int dummy; } audio_object_t;

audio_object_t* audio_object_open(int device, int sample_rate, int channels, int latency) {
    return NULL;
}

int audio_object_write(audio_object_t* obj, const short* samples, int count) {
    return -1;
}

const char* audio_object_strerror(int error) {
    return "audio backend not available (stub)";
}

int audio_object_close(audio_object_t* obj) {
    return 0;
}

int audio_object_flush(audio_object_t* obj) {
    return 0;
}

int audio_object_drain(audio_object_t* obj) {
    return 0;
}

void* create_audio_device_object(int device, int sample_rate) {
    return NULL;
}
