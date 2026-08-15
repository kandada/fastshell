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

typedef void (*aacode_event_fn)(const char *line, void *userdata);
extern void *aacode_task_start(const char *task_json, aacode_event_fn cb, void *userdata);
extern char *aacode_task_wait(void *handle);
extern void aacode_task_cancel(void *handle);
extern void aacode_task_free(void *handle);
extern char *aacode_validate_api_key(const char *config_json);
extern char *aacode_list_sessions(const char *project_path);
extern char *aacode_get_session_messages(const char *project_path, const char *session_id);
extern void aacode_free_string(char *ptr);

/* ── device capability callback (host → phone features) ────────────── */

typedef char *(*fastshell_device_callback)(const char *method, const char *args_json);
extern void fastshell_register_device_callback(fastshell_device_callback cb);

/* ── JavaVM + streaming callback state ─────────────────────────────── */

static JavaVM *g_vm = NULL;

/* Forward declaration (defined below with the per-task context registry). */
static void clear_all_task_ctx(void);

JNIEXPORT jint JNICALL JNI_OnLoad(JavaVM *vm, void *reserved) {
    (void)reserved;
    g_vm = vm;
    return JNI_VERSION_1_6;
}

JNIEXPORT void JNICALL JNI_OnUnload(JavaVM *vm, void *reserved) {
    (void)vm;
    (void)reserved;
    // Clear per-task callback contexts to prevent use-after-free on Rust worker
    // threads that might still be running during shutdown.
    clear_all_task_ctx();
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

/* ── Per-task callback context (userdata) for aacode_task_start ─────── */

typedef struct {
    jobject cb;      /* global ref to the Kotlin StreamCallback */
    jmethodID mid;   /* onChunk(Ljava/lang/String;)V             */
} jni_cb_ctx;

/*
 * Callback trampoline: Rust worker threads invoke this with the userdata we
 * supplied at aacode_task_start. We re-acquire a JNIEnv per thread (worker
 * threads attach once and stay attached) and forward the line to Java.
 */
static void event_trampoline(const char *line, void *userdata) {
    jni_cb_ctx *ctx = (jni_cb_ctx *)userdata;
    if (ctx == NULL || line == NULL || ctx->cb == NULL || ctx->mid == NULL) return;
    JNIEnv *env = get_env();
    if (env == NULL) return;

    jstring js = utf8_to_jstring(env, line);
    if (js == NULL) {
        if ((*env)->ExceptionCheck(env)) (*env)->ExceptionClear(env);
        return;
    }
    (*env)->CallVoidMethod(env, ctx->cb, ctx->mid, js);
    if ((*env)->ExceptionCheck(env)) (*env)->ExceptionClear(env);
    (*env)->DeleteLocalRef(env, js);
}

/* ── handle → ctx registry (for freeing callback contexts) ──────────── */

#define MAX_AGENT_TASKS 64

typedef struct {
    void *handle;
    jni_cb_ctx *ctx;
    int in_use;
} agent_task_slot;

static agent_task_slot g_agent_tasks[MAX_AGENT_TASKS];
static pthread_mutex_t g_agent_tasks_lock = PTHREAD_MUTEX_INITIALIZER;

static void register_task_ctx(void *handle, jni_cb_ctx *ctx) {
    pthread_mutex_lock(&g_agent_tasks_lock);
    for (int i = 0; i < MAX_AGENT_TASKS; i++) {
        if (!g_agent_tasks[i].in_use) {
            g_agent_tasks[i].handle = handle;
            g_agent_tasks[i].ctx = ctx;
            g_agent_tasks[i].in_use = 1;
            break;
        }
    }
    pthread_mutex_unlock(&g_agent_tasks_lock);
}

static jni_cb_ctx *unregister_task_ctx(void *handle) {
    jni_cb_ctx *ctx = NULL;
    pthread_mutex_lock(&g_agent_tasks_lock);
    for (int i = 0; i < MAX_AGENT_TASKS; i++) {
        if (g_agent_tasks[i].in_use && g_agent_tasks[i].handle == handle) {
            ctx = g_agent_tasks[i].ctx;
            g_agent_tasks[i].handle = NULL;
            g_agent_tasks[i].ctx = NULL;
            g_agent_tasks[i].in_use = 0;
            break;
        }
    }
    pthread_mutex_unlock(&g_agent_tasks_lock);
    return ctx;
}

/* Called on JNI_OnUnload: clear contexts without DeleteGlobalRef (the JVM is
 * already tearing down). */
static void clear_all_task_ctx(void) {
    pthread_mutex_lock(&g_agent_tasks_lock);
    for (int i = 0; i < MAX_AGENT_TASKS; i++) {
        if (g_agent_tasks[i].in_use) {
            g_agent_tasks[i].handle = NULL;
            g_agent_tasks[i].ctx = NULL;
            g_agent_tasks[i].in_use = 0;
        }
    }
    pthread_mutex_unlock(&g_agent_tasks_lock);
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
 * aacode-rs Native Agent JNI (handle-based)
 * ═══════════════════════════════════════════════════════════════════ */

/**
 * Start an agent task with a streaming callback (non-blocking). Returns the
 * opaque Rust task handle as a jlong (0 on catastrophic failure). Events stream
 * through the callback; call nativeAgentWaitTask to block for the result.
 */
JNIEXPORT jlong JNICALL
Java_com_fastshell_Sdk_nativeAgentRunTaskWithCallback(JNIEnv *env, jclass cls,
    jstring task_json, jobject callback) {
    (void)cls;
    char *json = jstring_to_utf8(env, task_json);

    jni_cb_ctx *ctx = (jni_cb_ctx *)calloc(1, sizeof(jni_cb_ctx));
    if (ctx == NULL) {
        free(json);
        return 0;
    }
    if (callback != NULL) {
        ctx->cb = (*env)->NewGlobalRef(env, callback);
        jclass cbcls = (*env)->GetObjectClass(env, callback);
        ctx->mid = (*env)->GetMethodID(env, cbcls, "onChunk", "(Ljava/lang/String;)V");
        (*env)->DeleteLocalRef(env, cbcls);
    }

    void *handle = aacode_task_start(json, ctx->mid ? event_trampoline : NULL, ctx);
    free(json);

    if (handle == NULL) {
        if (ctx->cb != NULL) (*env)->DeleteGlobalRef(env, ctx->cb);
        free(ctx);
        return 0;
    }
    register_task_ctx(handle, ctx);
    return (jlong)handle;
}

/** Block until the task finishes; returns the terminal JSON result string. */
JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeAgentWaitTask(JNIEnv *env, jclass cls, jlong handle) {
    (void)cls;
    if (handle == 0) return utf8_to_jstring(env, "{\"status\":\"error\",\"error\":\"null handle\"}");
    char *result = aacode_task_wait((void *)handle);
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

/* Cancel the task identified by its handle. */
JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeAgentCancelTask(JNIEnv *env, jclass cls, jlong handle) {
    (void)env; (void)cls;
    if (handle == 0) return;
    aacode_task_cancel((void *)handle);
}

/* Free a finished task handle + its callback context. */
JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeAgentFreeTask(JNIEnv *env, jclass cls, jlong handle) {
    (void)cls;
    if (handle == 0) return;
    jni_cb_ctx *ctx = unregister_task_ctx((void *)handle);
    if (ctx != NULL) {
        if (ctx->cb != NULL) (*env)->DeleteGlobalRef(env, ctx->cb);
        free(ctx);
    }
    aacode_task_free((void *)handle);
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
