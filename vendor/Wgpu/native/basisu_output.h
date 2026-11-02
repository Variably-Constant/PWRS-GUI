/*
 * Included ahead of every Basis Universal source build.rs compiles. Once
 * stdio.h is in, calls to the functions basisu writes its warnings and
 * errors with (printf, vprintf, fprintf and fputs) go to the functions in
 * basisu_output.c, which hand stdout and stderr text to the module instead
 * of the process's console. The standard headers' own declarations keep
 * their names: only calls in code compiled after this header change.
 */
#ifndef WGPU_BASISU_OUTPUT_H
#define WGPU_BASISU_OUTPUT_H

#include <stdarg.h>
#include <stdio.h>

#ifdef __cplusplus
#include <cstdio>
extern "C" {
#endif

int wgpu_basisu_printf(const char *format, ...);
int wgpu_basisu_vprintf(const char *format, va_list args);
int wgpu_basisu_fprintf(FILE *stream, const char *format, ...);
int wgpu_basisu_fputs(const char *text, FILE *stream);

#ifdef __cplusplus
}

/* basisu also calls std::printf, which the macros below turn into
   std::wgpu_basisu_printf: these names make that resolve. */
namespace std {
using ::wgpu_basisu_printf;
using ::wgpu_basisu_vprintf;
using ::wgpu_basisu_fprintf;
using ::wgpu_basisu_fputs;
}
#endif

/* Names, not function-like macros: basisu writes preprocessor directives
   inside some printf calls' arguments, which a function-like macro cannot
   take. */
#define printf wgpu_basisu_printf
#define vprintf wgpu_basisu_vprintf
#define fprintf wgpu_basisu_fprintf
#define fputs wgpu_basisu_fputs

#endif
