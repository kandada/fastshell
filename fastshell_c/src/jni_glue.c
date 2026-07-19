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
    jstring jc = (*ctx->env)->NewStringUTF(ctx->env, chunk);
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

    jstring jchunk = (*env)->NewStringUTF(env, chunk);
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
    const char *carg = (arg != NULL) ? (*env)->GetStringUTFChars(env, arg, NULL) : NULL;
    char *result = fn(carg);
    if (arg != NULL) {
        (*env)->ReleaseStringUTFChars(env, arg, carg);
    }
    jstring jresult = (*env)->NewStringUTF(env, result != NULL ? result : "");
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
    const char *cdir = (dir != NULL) ? (*env)->GetStringUTFChars(env, dir, NULL) : NULL;
    const char *ccmd = (command != NULL) ? (*env)->GetStringUTFChars(env, command, NULL) : NULL;
    char *result = fastshell_execute_in(cdir, ccmd);
    if (dir != NULL) (*env)->ReleaseStringUTFChars(env, dir, cdir);
    if (command != NULL) (*env)->ReleaseStringUTFChars(env, command, ccmd);
    jstring jresult = (*env)->NewStringUTF(env, result != NULL ? result : "");
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
    jstring jresult = (*env)->NewStringUTF(env, result != NULL ? result : "/");
    if (result != NULL) {
        fastshell_free_string(result);
    }
    return jresult;
}

JNIEXPORT void JNICALL
Java_com_fastshell_Sdk_nativeSetPermission(JNIEnv *env, jclass cls,
                                           jstring resource, jboolean allowed) {
    (void)cls;
    const char *cres = (resource != NULL) ? (*env)->GetStringUTFChars(env, resource, NULL) : NULL;
    fastshell_set_permission(cres, allowed ? 1 : 0);
    if (resource != NULL) {
        (*env)->ReleaseStringUTFChars(env, resource, cres);
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
    jstring jresult = (*env)->NewStringUTF(env, result ? result : "{\"ok\":false,\"error\":\"null result\"}");
    if (result) fastshell_free_string(result);
    return jresult;
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeSubmitTask(JNIEnv *env, jclass cls,
                                        jstring task_id, jstring task_json) {
    (void)cls;
    const char *cid = (*env)->GetStringUTFChars(env, task_id, NULL);
    const char *cjson = (*env)->GetStringUTFChars(env, task_json, NULL);
    char *result = fastshell_submit_task(cid, cjson);
    (*env)->ReleaseStringUTFChars(env, task_id, cid);
    (*env)->ReleaseStringUTFChars(env, task_json, cjson);
    jstring jresult = (*env)->NewStringUTF(env, result ? result : "{\"ok\":false,\"error\":\"null result\"}");
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

    jstring jmethod = (*env)->NewStringUTF(env, method ? method : "");
    jstring jargs = (*env)->NewStringUTF(env, args_json ? args_json : "{}");
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

    const char *utf = (*env)->GetStringUTFChars(env, jresult, NULL);
    char *out = strdup(utf ? utf : "{\"ok\":false,\"error\":\"utf\"}");
    if (utf) (*env)->ReleaseStringUTFChars(env, jresult, utf);
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
    const char *json = (*env)->GetStringUTFChars(env, task_json, NULL);
    char *result = aacode_run_task(json);
    (*env)->ReleaseStringUTFChars(env, task_json, json);
    jstring js = (*env)->NewStringUTF(env, result ? result : "{\"status\":\"error\",\"error\":\"null result\"}");
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
    const char *json = (*env)->GetStringUTFChars(env, task_json, NULL);

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

    (*env)->ReleaseStringUTFChars(env, task_json, json);
    jstring js = (*env)->NewStringUTF(env, result ? result : "{\"status\":\"error\",\"error\":\"null result\"}");
    if (result) aacode_free_string(result);
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
    const char *tid = (*env)->GetStringUTFChars(env, task_id, NULL);
    aacode_cancel_task(tid);
    (*env)->ReleaseStringUTFChars(env, task_id, tid);
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeAgentValidateApiKey(JNIEnv *env, jclass cls, jstring config_json) {
    (void)cls;
    const char *json = (*env)->GetStringUTFChars(env, config_json, NULL);
    char *result = aacode_validate_api_key(json);
    (*env)->ReleaseStringUTFChars(env, config_json, json);
    jstring js = (*env)->NewStringUTF(env, result ? result : "{\"valid\":false}");
    if (result) aacode_free_string(result);
    return js;
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeAgentListSessions(JNIEnv *env, jclass cls, jstring project_path) {
    (void)cls;
    const char *pp = (*env)->GetStringUTFChars(env, project_path, NULL);
    char *result = aacode_list_sessions(pp);
    (*env)->ReleaseStringUTFChars(env, project_path, pp);
    jstring js = (*env)->NewStringUTF(env, result ? result : "[]");
    if (result) aacode_free_string(result);
    return js;
}

JNIEXPORT jstring JNICALL
Java_com_fastshell_Sdk_nativeAgentGetSessionMessages(JNIEnv *env, jclass cls, jstring project_path, jstring session_id) {
    (void)cls;
    const char *pp = (*env)->GetStringUTFChars(env, project_path, NULL);
    const char *sid = (*env)->GetStringUTFChars(env, session_id, NULL);
    char *result = aacode_get_session_messages(pp, sid);
    (*env)->ReleaseStringUTFChars(env, project_path, pp);
    (*env)->ReleaseStringUTFChars(env, session_id, sid);
    jstring js = (*env)->NewStringUTF(env, result ? result : "[]");
    if (result) aacode_free_string(result);
    return js;
}
