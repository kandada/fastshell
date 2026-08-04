/*
 * Copyright (c) 2025 xiefujin <490021684@qq.com>
 * Licensed under Apache-2.0, see LICENSE file for full license terms.
 *
 * jni_glue.c — JNI <-> pure C ABI bridge (Android integration 方案 B).
 *
 * The Rust crate is compiled as a staticlib (libfastshell.a) that exports
 * only pure `extern "C"` symbols (see include/fastshell.h). This file
 * implements the `Java_com_fastshell_Sdk_native*` JNI entry points that
 * Kotlin's `com.fastshell.Sdk` declares, forwarding each call to the Rust
 * C ABI. CMake links this file + libfastshell.a with the NDK toolchain,
 * producing a standard-NDK-format libfastshell_jni.so that ART loads
 * without the Rust-cdylib ELF compatibility issues.
 *
 * Kotlin side is unchanged except System.loadLibrary("fastshell_jni").
 */

#include <jni.h>
#include <string.h>
#include <stdlib.h>
#include <pthread.h>
#include "fastshell.h"

/* ── UTF-8 <-> UTF-16 helpers (avoids JNI Modified UTF-8 corruption) ─ */

static jstring utf8_to_jstring(JNIEnv *env, const char *utf8) {
    if (utf8 == NULL) return NULL;
    size_t n = strlen(utf8);
    if (n == 0) return utf8_to_jstring(env, "");
    jsize out_len = 0;
    const unsigned char *p = (const unsigned char *)utf8;
    size_t i = 0;
    while (i < n) {
        if (p[i] < 0x80) { i += 1; out_len += 1; }
        else if ((p[i] & 0xE0) == 0xC0) { i += 2; out_len += 1; }
        else if ((p[i] & 0xF0) == 0xE0) { i += 3; out_len += 1; }
        else if ((p[i] & 0xF8) == 0xF0) { i += 4; out_len += 2; }
        else { i += 1; out_len += 1; }
    }
    jchar *buf = (jchar *)malloc((size_t)out_len * sizeof(jchar));
    if (buf == NULL) return utf8_to_jstring(env, utf8);
    p = (const unsigned char *)utf8;
    i = 0; jsize j = 0;
    while (i < n && j < out_len) {
        unsigned int cp;
        if (p[i] < 0x80) { cp = p[i]; i += 1; }
        else if ((p[i] & 0xE0) == 0xC0 && i + 1 < n) { cp = ((p[i] & 0x1F) << 6) | (p[i+1] & 0x3F); i += 2; }
        else if ((p[i] & 0xF0) == 0xE0 && i + 2 < n) { cp = ((p[i] & 0x0F) << 12) | ((p[i+1] & 0x3F) << 6) | (p[i+2] & 0x3F); i += 3; }
        else if ((p[i] & 0xF8) == 0xF0 && i + 3 < n) { cp = ((p[i] & 0x07) << 18) | ((p[i+1] & 0x3F) << 12) | ((p[i+2] & 0x3F) << 6) | (p[i+3] & 0x3F); i += 4; }
        else { cp = p[i]; i += 1; }
        if (cp <= 0xFFFF) { buf[j++] = (jchar)cp; }
        else { cp -= 0x10000; buf[j++] = (jchar)(0xD800 | (cp >> 10)); if (j < out_len) buf[j++] = (jchar)(0xDC00 | (cp & 0x3FF)); }
    }
    out_len = j;
    jstring result = (*env)->NewString(env, buf, out_len);
    free(buf);
    if (result != NULL) return result;
    return utf8_to_jstring(env, utf8);
}

static char *jstring_to_utf8(JNIEnv *env, jstring str) {
    if (str == NULL) return NULL;
    const jchar *chars = (*env)->GetStringChars(env, str, NULL);
    if (chars == NULL) {
        char *u = jstring_to_utf8(env, str);
        if (u == NULL) return NULL;
        char *result = strdup(u);
        free(u);
        return result;
    }
    jsize len = (*env)->GetStringLength(env, str);
    size_t cap = (size_t)len * 4 + 1;
    char *buf = (char *)malloc(cap);
    if (buf == NULL) { (*env)->ReleaseStringChars(env, str, chars); return NULL; }
    size_t j = 0;
    for (jsize i = 0; i < len; i++) {
        unsigned int cp = (unsigned int)chars[i];
        if (cp >= 0xD800 && cp <= 0xDBFF && i + 1 < len) {
            unsigned int lo = (unsigned int)chars[i+1];
            if (lo >= 0xDC00 && lo <= 0xDFFF) { cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00); i++; }
        }
        if (cp < 0x80) { buf[j++] = (char)cp; }
        else if (cp < 0x800) { buf[j++] = (char)(0xC0 | (cp >> 6)); buf[j++] = (char)(0x80 | (cp & 0x3F)); }
        else if (cp < 0x10000) { buf[j++] = (char)(0xE0 | (cp >> 12)); buf[j++] = (char)(0x80 | ((cp >> 6) & 0x3F)); buf[j++] = (char)(0x80 | (cp & 0x3F)); }
        else { buf[j++] = (char)(0xF0 | (cp >> 18)); buf[j++] = (char)(0x80 | ((cp >> 12) & 0x3F)); buf[j++] = (char)(0x80 | ((cp >> 6) & 0x3F)); buf[j++] = (char)(0x80 | (cp & 0x3F)); }
    }
    buf[j] = '\0';
    (*env)->ReleaseStringChars(env, str, chars);
    return buf;
}

/* ── aacode-rs native agent C ABI (exported from libaacode_rs.a) ────── */

typedef void (*aacode_stream_callback)(const char *line);
extern void aacode_register_stream_callback(aacode_stream_callback cb);
extern char *aacode_run_task(const char *task_json);
extern char *aacode_run_task_with_cb(const char *task_json, aacode_stream_callback cb);
extern void aacode_cancel(void);
extern void aacode_cancel_task(const char *task_id);
extern char *aacode_validate_api_key(const char *config_json);
extern char *aacode_list_sessions(const char *project_path);
extern char *aacode_get_session_messages(const char *project_path, const char *session_id);
extern void aacode_free_string(char *ptr);

/* ── device capability callback (host → phone features) ────────────── */

typedef char *(*fastshell_device_callback)(const char *method, const char *args_json);
extern void fastshell_register_device_callback(fastshell_device_callback cb);

/* ── JavaVM + streaming callback state ─────────────────────────────── */

static JavaVM *g_vm = NULL;
static jobject g_stream_cb = NULL;      /* global ref to StreamCallback   */
static jmethodID g_on_chunk = NULL;     /* onChunk(Ljava/lang/String;)V   */
static pthread_mutex_t g_cb_lock = PTHREAD_MUTEX_INITIALIZER;

JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM *vm, void *reserved) {
    (void)reserved;
    g_vm = vm;
    return JNI_VERSION_1_6;
}

JNIEXPORT void JNICALL JNI_OnUnload(JavaVM *vm, void *reserved) {
    (void)vm;
    (void)reserved;
    // Clear the stream callback to prevent use-after-free on Rust worker threads
    // that might still be running during shutdown.
    fastshell_register_stream_callback(NULL);
    pthread_mutex_lock(&g_cb_lock);
    if (g_stream_cb != NULL) {
        // We can't call DeleteGlobalRef here because JNI_OnUnload runs after
        // the JVM has started tearing down. Just clear our reference.
        g_stream_cb = NULL;
        g_on_chunk = NULL;
    }
    pthread_mutex_unlock(&g_cb_lock);
    g_vm = NULL;
}

/*
 * Acquire a JNIEnv for the current thread. Rust may invoke the stream
 * callback either on the calling JNI thread (already attached) or on an
 * internal worker thread (needs attach).
 *
 * Worker threads are attached ONCE and stay attached; a pthread TLS
 * destructor detaches them when the thread exits. This avoids the very
 * expensive AttachCurrentThread/DetachCurrentThread round-trip per
 * streamed token (hundreds per LLM response).
 */
static pthread_key_t g_env_key;
static pthread_once_t g_env_key_once = PTHREAD_ONCE_INIT;

static void detach_thread(void *value) {
    (void)value;
    if (g_vm != NULL) {
        (*g_vm)->DetachCurrentThread(g_vm);
    }
}

static void make_env_key(void) {
    pthread_key_create(&g_env_key, detach_thread);
}

static JNIEnv *get_env(void) {
    JNIEnv *env = NULL;
    if (g_vm == NULL) {
        return NULL;
    }
    jint rc = (*g_vm)->GetEnv(g_vm, (void **)&env, JNI_VERSION_1_6);
    if (rc == JNI_EDETACHED) {
        if ((*g_vm)->AttachCurrentThread(g_vm, &env, NULL) != JNI_OK) {
            return NULL;
        }
        /* Register the TLS destructor so the thread detaches on exit. */
        pthread_once(&g_env_key_once, make_env_key);
        pthread_setspecific(g_env_key, (void *)env);
    } else if (rc != JNI_OK) {
        return NULL;
    }
    return env;
}

/* ── Inline-trampoline context for aacode_run_task_with_cb ──────────── */

typedef struct {
    JNIEnv *env;
    jobject cb;
    jmethodID mid;
} stream_trampoline_ctx;

static pthread_key_t g_inline_ctx_key;
static pthread_once_t g_inline_ctx_once = PTHREAD_ONCE_INIT;

static void make_inline_ctx_key(void) {
    pthread_key_create(&g_inline_ctx_key, NULL);
}

static void set_inline_ctx(stream_trampoline_ctx *ctx) {
    pthread_once(&g_inline_ctx_once, make_inline_ctx_key);
    pthread_setspecific(g_inline_ctx_key, ctx);
}

static stream_trampoline_ctx *get_inline_ctx(void) {
    return (stream_trampoline_ctx *)pthread_getspecific(g_inline_ctx_key);
}

/* Per-task trampoline — uses thread-local ctx so concurrent tasks on
 * different threads don't interfere with each other. */
static void inline_trampoline(const char *chunk) {
    stream_trampoline_ctx *ctx = get_inline_ctx();
    if (ctx == NULL || chunk == NULL || ctx->cb == NULL || ctx->mid == NULL) return;
    jstring jc = utf8_to_jstring(ctx->env, chunk);
    if (jc == NULL) { if ((*ctx->env)->ExceptionCheck(ctx->env)) (*ctx->env)->ExceptionClear(ctx->env); return; }
    (*ctx->env)->CallVoidMethod(ctx->env, ctx->cb, ctx->mid, jc);
    if ((*ctx->env)->ExceptionCheck(ctx->env)) (*ctx->env)->ExceptionClear(ctx->env);
    (*ctx->env)->DeleteLocalRef(ctx->env, jc);
}

/* Trampoline registered with Rust; forwards each chunk to Java onChunk(). */
static void stream_trampoline(const char *chunk) {
    if (chunk == NULL || g_vm == NULL) {
        return;
    }
    pthread_mutex_lock(&g_cb_lock);
    jobject cb = g_stream_cb;
    jmethodID mid = g_on_chunk;
    pthread_mutex_unlock(&g_cb_lock);
    if (cb == NULL || mid == NULL) {
        return;
    }

    JNIEnv *env = get_env();
    if (env == NULL) {
        return;
    }

    jstring jchunk = utf8_to_jstring(env, chunk);
    if (jchunk == NULL) {
        // OutOfMemoryError — clear and continue
        if ((*env)->ExceptionCheck(env)) {
            (*env)->ExceptionClear(env);
        }
        return;
    }

    (*env)->CallVoidMethod(env, cb, mid, jchunk);
    if ((*env)->ExceptionCheck(env)) {
        (*env)->ExceptionClear(env);
    }
    (*env)->DeleteLocalRef(env, jchunk);
}

/* ── Helpers ───────────────────────────────────────────────────────── */

/* Calls a Rust fn(const char*) -> char* and returns a jstring, freeing
 * the Rust-owned buffer. Handles NULL results defensively. */
static jstring forward_str_in_str_out(JNIEnv *env, jstring arg,
                                      char *(*fn)(const char *)) {
    char *carg = (arg != NULL) ? jstring_to_utf8(env, arg) : NULL;
    char *result = fn(carg);
    if (arg != NULL) {
        free(carg);
    }
    jstring jresult = utf8_to_jstring(env, result != NULL ? result : "");
    if (result != NULL) {
        fastshell_free_string(result);
    }
    return jresult;
}

/* ── JNI entry points (names match com.fastshell.Sdk) ──────────────── */

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeInit(JNIEnv *env, jclass cls, jstring sandbox_path) {
    (void)cls;
    return forward_str_in_str_out(env, sandbox_path, fastshell_init);
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeExecute(JNIEnv *env, jclass cls, jstring command) {
    (void)cls;
    return forward_str_in_str_out(env, command, fastshell_execute);
}

/* Execute with an explicit working directory (restored afterwards). */
JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeExecuteIn(JNIEnv *env, jclass cls, jstring dir, jstring command) {
    (void)cls;
    char *cdir = (dir != NULL) ? jstring_to_utf8(env, dir) : NULL;
    char *ccmd = (command != NULL) ? jstring_to_utf8(env, command) : NULL;
    char *result = fastshell_execute_in(cdir, ccmd);
    if (dir != NULL) free(cdir);
    if (command != NULL) free(ccmd);
    jstring jresult = utf8_to_jstring(env, result != NULL ? result : "");
    if (result != NULL) fastshell_free_string(result);
    return jresult;
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeExecutePython(JNIEnv *env, jclass cls, jstring code) {
    (void)cls;
    return forward_str_in_str_out(env, code, fastshell_execute_python);
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeExecutePythonScript(JNIEnv *env, jclass cls, jstring script_path) {
    (void)cls;
    return forward_str_in_str_out(env, script_path, fastshell_execute_python_script);
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeGetCwd(JNIEnv *env, jclass cls) {
    (void)cls;
    char *result = fastshell_get_cwd();
    jstring jresult = utf8_to_jstring(env, result != NULL ? result : "/");
    if (result != NULL) {
        fastshell_free_string(result);
    }
    return jresult;
}

JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeSetPermission(JNIEnv *env, jclass cls,
                                           jstring resource, jboolean allowed) {
    (void)cls;
    char *cres = (resource != NULL) ? jstring_to_utf8(env, resource) : NULL;
    fastshell_set_permission(cres, allowed ? 1 : 0);
    if (resource != NULL) {
        free(cres);
    }
}

JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeCancelExecution(JNIEnv *env, jclass cls) {
    (void)env;
    (void)cls;
    fastshell_cancel_execution();
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeStartAgentServer(JNIEnv *env, jclass cls) {
    (void)cls;
    char *result = fastshell_start_agent_server();
    jstring jresult = utf8_to_jstring(env, result ? result : "{\"ok\":false,\"error\":\"null result\"}");
    if (result) fastshell_free_string(result);
    return jresult;
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeSubmitTask(JNIEnv *env, jclass cls,
                                        jstring task_id, jstring task_json) {
    (void)cls;
    char *cid = jstring_to_utf8(env, task_id);
    char *cjson = jstring_to_utf8(env, task_json);
    char *result = fastshell_submit_task(cid, cjson);
    free(cid);
    free(cjson);
    jstring jresult = utf8_to_jstring(env, result ? result : "{\"ok\":false,\"error\":\"null result\"}");
    if (result) fastshell_free_string(result);
    return jresult;
}

JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeRegisterStreamCallback(JNIEnv *env, jclass cls, jobject callback) {
    (void)cls;

    pthread_mutex_lock(&g_cb_lock);
    if (g_stream_cb != NULL) {
        (*env)->DeleteGlobalRef(env, g_stream_cb);
        g_stream_cb = NULL;
        g_on_chunk = NULL;
    }

    if (callback == NULL) {
        pthread_mutex_unlock(&g_cb_lock);
        fastshell_register_stream_callback(NULL);
        return;
    }

    jobject gref = (*env)->NewGlobalRef(env, callback);
    jclass cbcls = (*env)->GetObjectClass(env, callback);
    jmethodID mid = (*env)->GetMethodID(env, cbcls, "onChunk", "(Ljava/lang/String;)V");
    (*env)->DeleteLocalRef(env, cbcls);

    if (gref == NULL || mid == NULL) {
        if (gref != NULL) {
            (*env)->DeleteGlobalRef(env, gref);
        }
        pthread_mutex_unlock(&g_cb_lock);
        fastshell_register_stream_callback(NULL);
        return;
    }

    g_stream_cb = gref;
    g_on_chunk = mid;
    pthread_mutex_unlock(&g_cb_lock);

    fastshell_register_stream_callback(stream_trampoline);
}

/* ═══════════════════════════════════════════════════════════════════
 * Device capability bridge: fastshell (Rust) → Kotlin PluginRegistrar
 * ═══════════════════════════════════════════════════════════════════ */

static jclass g_plugin_cls = NULL;      /* global ref: PluginRegistrar    */
static jmethodID g_dispatch_mid = NULL; /* static dispatch(String,String)  */

/* Called by fastshell for every device command. Returns a malloc'd JSON
 * string (Rust frees it with free()). Never returns NULL unless fatal. */
static char *device_trampoline(const char *method, const char *args_json) {
    if (g_vm == NULL || g_plugin_cls == NULL || g_dispatch_mid == NULL) {
        return strdup("{\"ok\":false,\"error\":\"device bridge not registered\"}");
    }
    JNIEnv *env = get_env();
    if (env == NULL) {
        return strdup("{\"ok\":false,\"error\":\"no JNI env\"}");
    }

    jstring jmethod = utf8_to_jstring(env, method ? method : "");
    jstring jargs = utf8_to_jstring(env, args_json ? args_json : "{}");
    if (jmethod == NULL || jargs == NULL) {
        if ((*env)->ExceptionCheck(env)) (*env)->ExceptionClear(env);
        if (jmethod) (*env)->DeleteLocalRef(env, jmethod);
        if (jargs) (*env)->DeleteLocalRef(env, jargs);
        return strdup("{\"ok\":false,\"error\":\"oom\"}");
    }

    jstring jresult = (jstring)(*env)->CallStaticObjectMethod(
        env, g_plugin_cls, g_dispatch_mid, jmethod, jargs);
    (*env)->DeleteLocalRef(env, jmethod);
    (*env)->DeleteLocalRef(env, jargs);

    if ((*env)->ExceptionCheck(env)) {
        (*env)->ExceptionClear(env);
        if (jresult) (*env)->DeleteLocalRef(env, jresult);
        return strdup("{\"ok\":false,\"error\":\"device dispatch threw\"}");
    }
    if (jresult == NULL) {
        return strdup("{\"ok\":false,\"error\":\"device dispatch returned null\"}");
    }

    char *utf = jstring_to_utf8(env, jresult);
    char *out = strdup(utf ? utf : "{\"ok\":false,\"error\":\"utf\"}");
    if (utf) free(utf);
    (*env)->DeleteLocalRef(env, jresult);
    return out;
}

JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeRegisterDeviceCallback(JNIEnv *env, jclass cls) {
    (void)cls;
    /* Cache PluginRegistrar + its static dispatch(String,String):String. */
    jclass local = (*env)->FindClass(env, "com/aacode/app/bridge/PluginRegistrar");
    if (local == NULL) {
        if ((*env)->ExceptionCheck(env)) (*env)->ExceptionClear(env);
        return;
    }
    if (g_plugin_cls != NULL) {
        (*env)->DeleteGlobalRef(env, g_plugin_cls);
        g_plugin_cls = NULL;
    }
    g_plugin_cls = (jclass)(*env)->NewGlobalRef(env, local);
    (*env)->DeleteLocalRef(env, local);
    g_dispatch_mid = (*env)->GetStaticMethodID(
        env, g_plugin_cls, "dispatch",
        "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;");
    if (g_dispatch_mid == NULL) {
        if ((*env)->ExceptionCheck(env)) (*env)->ExceptionClear(env);
        return;
    }
    fastshell_register_device_callback(device_trampoline);
}

/* ═══════════════════════════════════════════════════════════════════
 * aacode-rs Native Agent JNI (replaces the CPython Python agent)
 * ═══════════════════════════════════════════════════════════════════ */
JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeAgentRegisterStreamCallback(JNIEnv *env, jclass cls, jobject callback) {
    (void)cls;
    pthread_mutex_lock(&g_cb_lock);
    if (g_stream_cb != NULL) {
        (*env)->DeleteGlobalRef(env, g_stream_cb);
        g_stream_cb = NULL;
        g_on_chunk = NULL;
    }
    if (callback == NULL) {
        pthread_mutex_unlock(&g_cb_lock);
        aacode_register_stream_callback(NULL);
        return;
    }
    jobject gref = (*env)->NewGlobalRef(env, callback);
    jclass cbcls = (*env)->GetObjectClass(env, callback);
    jmethodID mid = (*env)->GetMethodID(env, cbcls, "onChunk", "(Ljava/lang/String;)V");
    (*env)->DeleteLocalRef(env, cbcls);
    if (gref == NULL || mid == NULL) {
        if (gref != NULL) (*env)->DeleteGlobalRef(env, gref);
        pthread_mutex_unlock(&g_cb_lock);
        aacode_register_stream_callback(NULL);
        return;
    }
    g_stream_cb = gref;
    g_on_chunk = mid;
    pthread_mutex_unlock(&g_cb_lock);
    aacode_register_stream_callback(stream_trampoline);
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeAgentRunTask(JNIEnv *env, jclass cls, jstring task_json) {
    (void)cls;
    char *json = jstring_to_utf8(env, task_json);
    char *result = aacode_run_task(json);
    free(json);
    jstring js = utf8_to_jstring(env, result ? result : "{\"status\":\"error\",\"error\":\"null result\"}");
    if (result) aacode_free_string(result);
    return js;
}

/**
 * Run an agent task WITH an inline streaming callback — supports concurrent
 * tasks because each call carries its own callback, avoiding the global
 * callback slot conflict of the older register-then-run API.
 */
JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeAgentRunTaskWithCallback(JNIEnv *env, jclass cls,
    jstring task_json, jobject callback) {
    (void)cls;
    char *json = jstring_to_utf8(env, task_json);

    stream_trampoline_ctx ctx;
    ctx.env = env;
    ctx.cb = callback;
    ctx.mid = NULL;
    if (callback != NULL) {
        jclass cbcls = (*env)->GetObjectClass(env, callback);
        ctx.mid = (*env)->GetMethodID(env, cbcls, "onChunk", "(Ljava/lang/String;)V");
        (*env)->DeleteLocalRef(env, cbcls);
    }
    set_inline_ctx(&ctx);

    char *result = aacode_run_task_with_cb(json, ctx.mid ? inline_trampoline : NULL);
    set_inline_ctx(NULL);

    free(json);
    jstring js = utf8_to_jstring(env, result ? result : "{\"status\":\"error\",\"error\":\"null result\"}");
    if (result) aacode_free_string(result);
    return js;
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeGetFeatures(JNIEnv *env, jclass cls) {
    (void)cls;
    char *result = fastshell_get_features();
    jstring js = utf8_to_jstring(env, result ? result : "{}");
    if (result) fastshell_free_string(result);
    return js;
}

JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeAgentCancel(JNIEnv *env, jclass cls) {
    (void)env; (void)cls;
    aacode_cancel();
}

/* Cancel ONLY the task registered with this client_task_id. */
JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeAgentCancelTask(JNIEnv *env, jclass cls, jstring task_id) {
    (void)cls;
    if (task_id == NULL) return;
    char *tid = jstring_to_utf8(env, task_id);
    aacode_cancel_task(tid);
    free(tid);
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeAgentValidateApiKey(JNIEnv *env, jclass cls, jstring config_json) {
    (void)cls;
    char *json = jstring_to_utf8(env, config_json);
    char *result = aacode_validate_api_key(json);
    free(json);
    jstring js = utf8_to_jstring(env, result ? result : "{\"valid\":false}");
    if (result) aacode_free_string(result);
    return js;
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeAgentListSessions(JNIEnv *env, jclass cls, jstring project_path) {
    (void)cls;
    char *pp = jstring_to_utf8(env, project_path);
    char *result = aacode_list_sessions(pp);
    free(pp);
    jstring js = utf8_to_jstring(env, result ? result : "[]");
    if (result) aacode_free_string(result);
    return js;
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeAgentGetSessionMessages(JNIEnv *env, jclass cls, jstring project_path, jstring session_id) {
    (void)cls;
    char *pp = jstring_to_utf8(env, project_path);
    char *sid = jstring_to_utf8(env, session_id);
    char *result = aacode_get_session_messages(pp, sid);
    free(pp);
    free(sid);
    jstring js = utf8_to_jstring(env, result ? result : "[]");
    if (result) aacode_free_string(result);
    return js;
}
