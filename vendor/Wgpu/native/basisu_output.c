/*
 * The functions basisu_output.h sends Basis Universal's printf, vprintf,
 * fprintf and fputs calls to. Each formats its text and passes text meant
 * for stdout or stderr to wgpu_basisu_text, in the module, with whether it
 * was stderr, where basisu writes its errors; text for any other stream is
 * written to that stream as before.
 */
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

void wgpu_basisu_text(const char *text, size_t length, int is_error);

static const char too_long[] = "Basis Universal printed a line the module had no memory to hold";

static int forward(FILE *stream, const char *format, va_list args)
{
    char small[512];
    va_list copy;
    va_copy(copy, args);
    int length = vsnprintf(small, sizeof small, format, copy);
    va_end(copy);
    if (length < 0) {
        return length;
    }
    if ((size_t)length < sizeof small) {
        wgpu_basisu_text(small, (size_t)length, stream == stderr);
        return length;
    }
    char *large = malloc((size_t)length + 1);
    if (large == NULL) {
        wgpu_basisu_text(too_long, sizeof too_long - 1, 1);
        return -1;
    }
    vsnprintf(large, (size_t)length + 1, format, args);
    wgpu_basisu_text(large, (size_t)length, stream == stderr);
    free(large);
    return length;
}

int wgpu_basisu_vprintf(const char *format, va_list args)
{
    return forward(stdout, format, args);
}

int wgpu_basisu_printf(const char *format, ...)
{
    va_list args;
    va_start(args, format);
    int length = forward(stdout, format, args);
    va_end(args);
    return length;
}

int wgpu_basisu_fprintf(FILE *stream, const char *format, ...)
{
    va_list args;
    va_start(args, format);
    int length = (stream == stdout || stream == stderr) ? forward(stream, format, args) : vfprintf(stream, format, args);
    va_end(args);
    return length;
}

int wgpu_basisu_fputs(const char *text, FILE *stream)
{
    if (stream != stdout && stream != stderr) {
        return fputs(text, stream);
    }
    wgpu_basisu_text(text, strlen(text), stream == stderr);
    return 0;
}
