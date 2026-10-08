/*
 * Copyright (c) 2026 xiefujin <490021684@qq.com>
 * Licensed under GPL-3.0, see LICENSE file for full license terms.
 *
 * jni_surface_bridge.c — Android native-accessibility surface bridge for aacode's
 * desktop/`ax_*` tools (compiled into libfastshell_jni.so alongside jni_glue.c).
 *
 *   Kotlin: com.aacode.app.surface.FastbrowserSurfaceBridge (static methods,
 *           backed by com.aacode.app.surface.FastbrowserAccessibilityService)
 *      ↑ JNI
 *   this file: FbSurfaceOps table (callbacks → Kotlin static methods, JSON)
 *      ↑
 *   aacode-rs: aacode_browser_register_surface_ops(&ops)
 *
 * The host app calls FastbrowserSurfaceBridge.register() once at startup, and the
 * user must enable the AccessibilityService in system settings.
 */

#include <jni.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

/* Mirror of fastbrowser's FbSurfaceOps (kept inline so the build needs no extra
 * header). All tree/action payloads cross as UTF-8 JSON strings. */
typedef struct FbSurfaceOps {
    char *(*capabilities)(void);
    char *(*list)(void);
    char *(*snapshot)(const char *target, const char *opts_json);
    char *(*act)(const char *ref, const char *action_json);
    int (*input)(const char *target, const char *event_json);
    char *(*screenshot)(const char *target);
    char *(*poll_events)(const char *target);
    void (*free_string)(char *ptr);
} FbSurfaceOps;

extern int aacode_browser_register_surface_ops(const FbSurfaceOps *ops);

/* ── cached Java handles ─────────────────────────────────────────── */

static JavaVM *g_vm = NULL;
static jclass g_sb = NULL;
static jmethodID m_capabilities = NULL, m_list = NULL, m_snapshot = NULL;
static jmethodID m_act = NULL, m_input = NULL, m_screenshot = NULL, m_poll_events = NULL;

static jstring js_new(JNIEnv *env, const char *s) {
    return (*env)->NewStringUTF(env, s ? s : "");
}

static JNIEnv *env_get(void) {
    JNIEnv *env = NULL;
    if (g_vm == NULL) return NULL;
    if ((*g_vm)->GetEnv(g_vm, (void **)&env, JNI_VERSION_1_6) != JNI_OK) {
        if ((*g_vm)->AttachCurrentThread(g_vm, (JNIEnv **)&env, NULL) != JNI_OK) return NULL;
    }
    return env;
}

/* Call a static method returning String; return strdup'd UTF-8 (never NULL). */
static char *call_str(jmethodID m) {
    JNIEnv *env = env_get();
    if (env == NULL || g_sb == NULL || m == NULL) return strdup("");
    jstring out = (jstring)(*env)->CallStaticObjectMethod(env, g_sb, m);
    if (out == NULL) return strdup("");
    const char *c = (*env)->GetStringUTFChars(env, out, NULL);
    char *res = strdup(c ? c : "");
    if (c) (*env)->ReleaseStringUTFChars(env, out, c);
    (*env)->DeleteLocalRef(env, out);
    return res;
}

static char *call_str2(jmethodID m, const char *a, const char *b) {
    JNIEnv *env = env_get();
    if (env == NULL || g_sb == NULL || m == NULL) return strdup("");
    jstring ja = js_new(env, a);
    jstring jb = js_new(env, b);
    jstring out = (jstring)(*env)->CallStaticObjectMethod(env, g_sb, m, ja, jb);
    (*env)->DeleteLocalRef(env, ja);
    (*env)->DeleteLocalRef(env, jb);
    if (out == NULL) return strdup("");
    const char *c = (*env)->GetStringUTFChars(env, out, NULL);
    char *res = strdup(c ? c : "");
    if (c) (*env)->ReleaseStringUTFChars(env, out, c);
    (*env)->DeleteLocalRef(env, out);
    return res;
}

static char *call_str1(jmethodID m, const char *a) {
    JNIEnv *env = env_get();
    if (env == NULL || g_sb == NULL || m == NULL) return strdup("");
    jstring ja = js_new(env, a);
    jstring out = (jstring)(*env)->CallStaticObjectMethod(env, g_sb, m, ja);
    (*env)->DeleteLocalRef(env, ja);
    if (out == NULL) return strdup("");
    const char *c = (*env)->GetStringUTFChars(env, out, NULL);
    char *res = strdup(c ? c : "");
    if (c) (*env)->ReleaseStringUTFChars(env, out, c);
    (*env)->DeleteLocalRef(env, out);
    return res;
}

/* ── FbSurfaceOps callbacks ──────────────────────────────────────── */

static char *op_capabilities(void) { return call_str(m_capabilities); }
static char *op_list(void) { return call_str(m_list); }
static char *op_snapshot(const char *t, const char *o) { return call_str2(m_snapshot, t, o); }
static char *op_act(const char *r, const char *a) { return call_str2(m_act, r, a); }
static char *op_screenshot(const char *t) { return call_str1(m_screenshot, t); }
static char *op_poll_events(const char *t) { return call_str1(m_poll_events, t); }

static int op_input(const char *t, const char *e) {
    JNIEnv *env = env_get();
    if (env == NULL || g_sb == NULL || m_input == NULL) return -1;
    jstring jt = js_new(env, t);
    jstring je = js_new(env, e);
    int r = (*env)->CallStaticIntMethod(env, g_sb, m_input, jt, je);
    (*env)->DeleteLocalRef(env, jt);
    (*env)->DeleteLocalRef(env, je);
    return r;
}

static void op_free_string(char *ptr) { free(ptr); }

/* ── registration entry (called from Kotlin) ─────────────────────── */

JNIEXPORT jint JNICALL
Java_com_aacode_app_surface_FastbrowserSurfaceBridge_nativeRegisterSurfaceOps(JNIEnv *env, jclass cls) {
    (void)cls;
    if (g_sb == NULL) {
        jclass c = (*env)->FindClass(env, "com/aacode/app/surface/FastbrowserSurfaceBridge");
        if (c == NULL) return -1;
        g_sb = (jclass)(*env)->NewGlobalRef(env, c);
        if (g_sb == NULL) return -1;
        m_capabilities = (*env)->GetStaticMethodID(env, g_sb, "capabilities", "()Ljava/lang/String;");
        m_list = (*env)->GetStaticMethodID(env, g_sb, "list", "()Ljava/lang/String;");
        m_snapshot = (*env)->GetStaticMethodID(env, g_sb, "snapshot", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;");
        m_act = (*env)->GetStaticMethodID(env, g_sb, "act", "(Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;");
        m_input = (*env)->GetStaticMethodID(env, g_sb, "input", "(Ljava/lang/String;Ljava/lang/String;)I");
        m_screenshot = (*env)->GetStaticMethodID(env, g_sb, "screenshot", "(Ljava/lang/String;)Ljava/lang/String;");
        m_poll_events = (*env)->GetStaticMethodID(env, g_sb, "pollEvents", "(Ljava/lang/String;)Ljava/lang/String;");
        if (g_vm == NULL) (*env)->GetJavaVM(env, &g_vm);
        if (!m_capabilities || !m_list || !m_snapshot || !m_act || !m_input ||
            !m_screenshot || !m_poll_events) {
            return -1;
        }
    }

    static FbSurfaceOps ops;
    memset(&ops, 0, sizeof(ops));
    ops.capabilities = op_capabilities;
    ops.list = op_list;
    ops.snapshot = op_snapshot;
    ops.act = op_act;
    ops.input = op_input;
    ops.screenshot = op_screenshot;
    ops.poll_events = op_poll_events;
    ops.free_string = op_free_string;
    return (jint)aacode_browser_register_surface_ops(&ops);
}
