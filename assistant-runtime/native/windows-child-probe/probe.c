// Ofox's fixed Windows sandbox probe uses the upstream Node-API C interface.
#define WIN32_LEAN_AND_MEAN
#define NAPI_VERSION 8
#include <windows.h>
#include <node_api.h>
#include <string.h>

// Bind only the public Node-API exports in this already-loaded host image.
// Resolve this running host directly without a node.exe image-name import.
// No module name, path, or export name can be supplied by the caller.
static struct {
    napi_status (NAPI_CDECL *throw_error)(napi_env, const char *, const char *);
    napi_status (NAPI_CDECL *get_boolean)(napi_env, bool, napi_value *);
    napi_status (NAPI_CDECL *set_named_property)(napi_env, napi_value, const char *, napi_value);
    napi_status (NAPI_CDECL *create_uint32)(napi_env, uint32_t, napi_value *);
    napi_status (NAPI_CDECL *get_cb_info)(napi_env, napi_callback_info, size_t *, napi_value *, napi_value *, void **);
    napi_status (NAPI_CDECL *create_object)(napi_env, napi_value *);
    napi_status (NAPI_CDECL *create_function)(napi_env, const char *, size_t, napi_callback, void *, napi_value *);
} node_api;

static bool load_node_api(void) {
    HMODULE host = GetModuleHandleW(NULL);
    if (!host) return false;
#define LOAD(field) do { \
    FARPROC address = GetProcAddress(host, "napi_" #field); \
    _Static_assert(sizeof(address) == sizeof(node_api.field), "Node-API pointer ABI"); \
    if (!address) return false; \
    memcpy(&node_api.field, &address, sizeof(address)); \
} while (0)
    LOAD(throw_error);
    LOAD(get_boolean);
    LOAD(set_named_property);
    LOAD(create_uint32);
    LOAD(get_cb_info);
    LOAD(create_object);
    LOAD(create_function);
#undef LOAD
    return true;
}

#define napi_throw_error node_api.throw_error
#define napi_get_boolean node_api.get_boolean
#define napi_set_named_property node_api.set_named_property
#define napi_create_uint32 node_api.create_uint32
#define napi_get_cb_info node_api.get_cb_info
#define napi_create_object node_api.create_object
#define napi_create_function node_api.create_function

static napi_value probe_error(napi_env env, const char *code, const char *message) {
    napi_throw_error(env, code, message);
    return NULL;
}

static bool set_flag(napi_env env, napi_value object, const char *name, bool flag) {
    napi_value value;
    return napi_get_boolean(env, flag, &value) == napi_ok &&
           napi_set_named_property(env, object, name, value) == napi_ok;
}

static bool set_u32(napi_env env, napi_value object, const char *name, uint32_t number) {
    napi_value value;
    return napi_create_uint32(env, number, &value) == napi_ok &&
           napi_set_named_property(env, object, name, value) == napi_ok;
}

static bool cleanup_child(BOOL created, const PROCESS_INFORMATION *child) {
    bool terminated = false;
    bool waited = false;
    bool process_closed = false;
    if (child->hProcess && child->hProcess != INVALID_HANDLE_VALUE) {
        terminated = TerminateProcess(child->hProcess, 1) != FALSE;
        waited = WaitForSingleObject(child->hProcess, 5000) == WAIT_OBJECT_0;
        process_closed = CloseHandle(child->hProcess) != FALSE;
    }
    bool thread_closed = child->hThread && child->hThread != INVALID_HANDLE_VALUE &&
                         CloseHandle(child->hThread) != FALSE;

    // Reclaim any returned handles, then require a canonical result for proof.
    if (!created) {
        return !child->hProcess && !child->hThread && !child->dwProcessId && !child->dwThreadId;
    }
    return child->dwProcessId && child->dwThreadId &&
           terminated && waited && process_closed && thread_closed;
}

static napi_value probe(napi_env env, napi_callback_info info) {
    size_t argc = 1;
    napi_value ignored;
    if (napi_get_cb_info(env, info, &argc, &ignored, NULL, NULL) != napi_ok || argc != 0) {
        return probe_error(env, "ERR_PROBE_ARGUMENTS", "Fixed probe takes no arguments");
    }

    wchar_t image[32768];
    DWORD length = GetModuleFileNameW(NULL, image, ARRAYSIZE(image));
    if (length == 0 || length == ARRAYSIZE(image)) {
        return probe_error(env, "ERR_PROBE_IMAGE", "Cannot identify current executable");
    }

    wchar_t command[] =
        L"node.exe -e \"require('node:fs').writeFileSync('child-marker', 'bad')\"";
    STARTUPINFOW startup = {0};
    PROCESS_INFORMATION child = {0};
    startup.cb = sizeof(startup);
    BOOL created = CreateProcessW(image, command, NULL, NULL, FALSE, CREATE_SUSPENDED,
                                  NULL, NULL, &startup, &child);
    DWORD error = created ? ERROR_SUCCESS : GetLastError();
    // Retain the result metadata before disposing of an unexpected child.
    bool process_handle = child.hProcess != NULL;
    bool thread_handle = child.hThread != NULL;
    bool cleanup_complete = cleanup_child(created, &child);

    napi_value value;
    if (napi_create_object(env, &value) != napi_ok ||
        !set_flag(env, value, "created", created != FALSE) ||
        !set_u32(env, value, "error", error) ||
        !set_u32(env, value, "processId", child.dwProcessId) ||
        !set_u32(env, value, "threadId", child.dwThreadId) ||
        !set_flag(env, value, "processHandle", process_handle) ||
        !set_flag(env, value, "threadHandle", thread_handle) ||
        !set_flag(env, value, "cleanupComplete", cleanup_complete)) {
        return probe_error(env, "ERR_PROBE_RESULT", "Cannot return fixed probe result");
    }
    return value;
}

static napi_value init(napi_env env, napi_value exports) {
    if (!load_node_api()) return NULL;
    napi_value function;
    if (napi_create_function(env, "probe", NAPI_AUTO_LENGTH, probe, NULL, &function) != napi_ok ||
        napi_set_named_property(env, exports, "probe", function) != napi_ok) {
        return probe_error(env, "ERR_PROBE_INIT", "Cannot initialize fixed probe");
    }
    return exports;
}

NAPI_MODULE(ofox_windows_child_probe, init)
