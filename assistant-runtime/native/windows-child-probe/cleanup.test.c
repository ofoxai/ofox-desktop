#define WIN32_LEAN_AND_MEAN
#define NAPI_VERSION 8
#include <windows.h>
#include <node_api.h>
#include <stdio.h>

static const HANDLE process = (HANDLE)(ULONG_PTR)0x1234;
static const HANDLE thread = (HANDLE)(ULONG_PTR)0x5678;

static struct {
    unsigned int terminates;
    unsigned int waits;
    unsigned int process_closes;
    unsigned int thread_closes;
    unsigned int unexpected_calls;
    bool fail_cleanup;
} calls;

static BOOL mock_terminate(HANDLE handle, UINT exit_code) {
    if (handle != process || exit_code != 1) {
        calls.unexpected_calls++;
        return FALSE;
    }
    calls.terminates++;
    return !calls.fail_cleanup;
}

static DWORD mock_wait(HANDLE handle, DWORD timeout) {
    if (handle != process || timeout != 5000) {
        calls.unexpected_calls++;
        return WAIT_FAILED;
    }
    calls.waits++;
    return calls.fail_cleanup ? WAIT_TIMEOUT : WAIT_OBJECT_0;
}

static BOOL mock_close(HANDLE handle) {
    if (handle == process) {
        calls.process_closes++;
    } else if (handle == thread) {
        calls.thread_closes++;
    } else {
        calls.unexpected_calls++;
        return FALSE;
    }
    return !calls.fail_cleanup;
}

// Test the real cleanup routine with synthetic handles. Omit addon registration.
#define TerminateProcess mock_terminate
#define WaitForSingleObject mock_wait
#define CloseHandle mock_close
#undef NAPI_MODULE
#define NAPI_MODULE(name, init)
#pragma warning(push)
#pragma warning(disable: 4505) // The unused Node-API entrypoint is not part of this test.
#include "probe.c"
#pragma warning(pop)

static bool called(unsigned int terminate, unsigned int wait,
                   unsigned int process_close, unsigned int thread_close) {
    return calls.terminates == terminate && calls.waits == wait &&
           calls.process_closes == process_close && calls.thread_closes == thread_close &&
           calls.unexpected_calls == 0;
}

#define EXPECT(expression)                                                             \
    do {                                                                              \
        if (!(expression)) {                                                          \
            fprintf(stderr, "cleanup test line %d: %s\n", __LINE__, #expression);      \
            return 1;                                                                 \
        }                                                                             \
    } while (0)

int main(void) {
    PROCESS_INFORMATION child = {0};
    EXPECT(cleanup_child(FALSE, &child) && called(0, 0, 0, 0));

    child.dwProcessId = 7;
    EXPECT(!cleanup_child(FALSE, &child) && called(0, 0, 0, 0));

    child.hProcess = process;
    child.hThread = thread;
    child.dwThreadId = 8;
    EXPECT(!cleanup_child(FALSE, &child) && called(1, 1, 1, 1));

    ZeroMemory(&calls, sizeof(calls));
    EXPECT(cleanup_child(TRUE, &child) && called(1, 1, 1, 1));

    ZeroMemory(&calls, sizeof(calls));
    calls.fail_cleanup = true;
    EXPECT(!cleanup_child(TRUE, &child) && called(1, 1, 1, 1));

    ZeroMemory(&calls, sizeof(calls));
    child.hProcess = INVALID_HANDLE_VALUE;
    EXPECT(!cleanup_child(FALSE, &child) && called(0, 0, 0, 1));

    ZeroMemory(&calls, sizeof(calls));
    child.hProcess = NULL;
    child.dwProcessId = 0;
    EXPECT(!cleanup_child(FALSE, &child) && called(0, 0, 0, 1));

    puts("Windows child probe cleanup tests passed (7/7)");
    return 0;
}
