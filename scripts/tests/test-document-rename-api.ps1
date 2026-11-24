#Requires -Version 7.0
# Diagnose the native rename API boundary before the full Rust build. Each
# combination owns invented files in one fresh temporary directory. A rejected
# combination is evidence, not a verification failure or a production fallback.
# Contracts: https://learn.microsoft.com/windows/win32/api/winbase/ns-winbase-file_rename_info
# https://learn.microsoft.com/windows-hardware/drivers/ddi/ntifs/ns-ntifs-_file_rename_information
Set-StrictMode -Version Latest
$ErrorActionPreference = "Stop"
if (-not $IsWindows) { throw "Native Windows is required for rename diagnostics." }

Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
using Microsoft.Win32.SafeHandles;

public static class OfoxDocumentRenameApi {
    public static string CleanupErrorType;

    [StructLayout(LayoutKind.Sequential)]
    struct IoStatusBlock { public IntPtr Status; public UIntPtr Information; }

    public sealed class Result {
        public string Api;
        public string ParentAccess;
        public string BufferRule;
        public int BufferBytes;
        public bool RenameSucceeded;
        public int Win32Error;
        public string NtStatus;
        public string IoStatus;
        public string FailedPhase;
        public string ManagedErrorType;
        public bool DestinationExists;
        public bool DestinationBytesMatch;
    }

    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern SafeFileHandle CreateFileW(string path, uint access, uint sharing,
        IntPtr security, uint disposition, uint flags, IntPtr template);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool SetFileInformationByHandle(SafeFileHandle file, int cls,
        IntPtr info, uint length);
    [DllImport("ntdll.dll")]
    static extern int NtSetInformationFile(SafeFileHandle file, out IoStatusBlock status,
        IntPtr info, uint length, int cls);
    [DllImport("ntdll.dll")]
    static extern uint RtlNtStatusToDosError(int status);

    // x64 SDK/windows-rs layout: union at 0, HANDLE at 8, ULONG at 16,
    // WCHAR[1] at 20, sizeof 24. Do not substitute Win32 class 3 for NT class 10.
    static Result RunCase(string root, bool native, bool traverse, bool conservative) {
        var row = new Result {
            Api = native ? "NtSetInformationFile-class10" : "SetFileInformationByHandle-class3",
            ParentAccess = traverse ? "FILE_TRAVERSE|FILE_READ_ATTRIBUTES" : "FILE_GENERIC_READ",
            BufferRule = conservative ? "sizeof-plus-name" : "max-sizeof-offset-plus-name"
        };
        string tag = (native ? "nt" : "win32") + "-" + (traverse ? "traverse" : "read")
            + "-" + (conservative ? "plus" : "current");
        string folder = Path.Combine(root, tag);
        string destination = Path.Combine(folder, "a");
        byte[] expected = Encoding.UTF8.GetBytes("synthetic inert document output");
        string phase = "create-case-directory";
        try {
            Directory.CreateDirectory(folder);
            phase = "open-parent";
            // Keep the parent open without FILE_SHARE_DELETE through the rename.
            using (var parent = CreateFileW(folder, traverse ? 0xA0u : 0x120089u, 3,
                IntPtr.Zero, 3, 0x02200000, IntPtr.Zero)) {
                if (parent.IsInvalid) {
                    row.Win32Error = Marshal.GetLastWin32Error();
                    row.FailedPhase = phase;
                    return row;
                }
                phase = "create-source";
                using (var file = CreateFileW(Path.Combine(folder, ".owned-source"),
                    0x120089u | 0x120116u | 0x10000u, 1, IntPtr.Zero, 1,
                    0x00200000, IntPtr.Zero)) {
                    if (file.IsInvalid) {
                        row.Win32Error = Marshal.GetLastWin32Error();
                        row.FailedPhase = phase;
                        return row;
                    }
                    using (var stream = new FileStream(file, FileAccess.ReadWrite, 4096, false)) {
                        phase = "write-and-fsync";
                        stream.Write(expected, 0, expected.Length);
                        stream.Flush(true);
                        byte[] name = Encoding.Unicode.GetBytes("a");
                        int length = conservative ? 24 + name.Length : Math.Max(24, 20 + name.Length);
                        row.BufferBytes = length;
                        IntPtr memory = Marshal.AllocHGlobal(length);
                        try {
                            for (int i = 0; i < length; i++) Marshal.WriteByte(memory, i, 0);
                            Marshal.WriteInt32(memory, 0, 1); // ReplaceIfExists; rest of union is zero.
                            Marshal.WriteIntPtr(memory, 8, parent.DangerousGetHandle());
                            Marshal.WriteInt32(memory, 16, name.Length);
                            Marshal.Copy(name, 0, IntPtr.Add(memory, 20), name.Length);
                            phase = "rename";
                            if (native) {
                                IoStatusBlock io;
                                int status = NtSetInformationFile(file, out io, memory, (uint)length, 10);
                                row.NtStatus = unchecked((uint)status).ToString("x8");
                                row.IoStatus = unchecked((uint)io.Status.ToInt64()).ToString("x8");
                                row.Win32Error = unchecked((int)RtlNtStatusToDosError(status));
                                row.RenameSucceeded = status == 0;
                            } else {
                                row.RenameSucceeded = SetFileInformationByHandle(file, 3, memory, (uint)length);
                                row.Win32Error = row.RenameSucceeded ? 0 : Marshal.GetLastWin32Error();
                            }
                            if (!row.RenameSucceeded) row.FailedPhase = phase;
                        } finally { Marshal.FreeHGlobal(memory); }
                    }
                }
            }
            phase = "read-owned-destination";
            row.DestinationExists = File.Exists(destination);
            if (row.DestinationExists) {
                byte[] actual = File.ReadAllBytes(destination);
                row.DestinationBytesMatch = Convert.ToBase64String(actual) == Convert.ToBase64String(expected);
            }
        } catch (Exception error) {
            row.FailedPhase = phase;
            // Exceptions can contain personal temporary paths; emit only type.
            row.ManagedErrorType = error.GetType().Name;
        }
        return row;
    }

    public static Result[] Run() {
        if (IntPtr.Size != 8) throw new PlatformNotSupportedException("The observed layout is x64 only.");
        CleanupErrorType = null;
        string root = Path.Combine(Path.GetTempPath(), "ofox-rename-diagnostic-" + Guid.NewGuid().ToString("N"));
        Directory.CreateDirectory(root);
        try {
            var rows = new List<Result>();
            foreach (bool native in new[] { false, true })
                foreach (bool traverse in new[] { false, true })
                    foreach (bool conservative in new[] { false, true })
                        rows.Add(RunCase(root, native, traverse, conservative));
            return rows.ToArray();
        } finally {
            // Only this invocation's invented directory is removed.
            // A cleanup error must not discard the eight diagnostic rows or
            // print an exception containing a personal temporary path.
            try { Directory.Delete(root, true); }
            catch (Exception error) { CleanupErrorType = error.GetType().Name; }
        }
    }
}
'@

[pscustomobject]@{
    diagnosticOnly = $true
    nativeOsVersion = [Environment]::OSVersion.Version.ToString()
    pointerBytes = [IntPtr]::Size
    renameOffsets = @(0, 8, 16, 20)
    renameStructBytes = 24
} | ConvertTo-Json -Compress | Write-Host
try {
    $renameRows = [OfoxDocumentRenameApi]::Run()
} catch {
    [pscustomobject]@{
        diagnosticOnly = $true
        phase = "temporary-root-setup"
        managedErrorType = $_.Exception.GetBaseException().GetType().Name
    } | ConvertTo-Json -Compress | Write-Host
    return
}
foreach ($renameResult in $renameRows) {
    $renameResult | ConvertTo-Json -Compress | Write-Host
}
if ([OfoxDocumentRenameApi]::CleanupErrorType) {
    [pscustomobject]@{
        diagnosticOnly = $true
        phase = "temporary-root-cleanup"
        managedErrorType = [OfoxDocumentRenameApi]::CleanupErrorType
    } | ConvertTo-Json -Compress | Write-Host
}
