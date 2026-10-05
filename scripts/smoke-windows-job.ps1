[CmdletBinding()]
param([Parameter(Mandatory)][string]$Configuration)
$ErrorActionPreference = 'Stop'
# The smoke CLI supplies a private configuration file. Create the application
# suspended, assign it to a kill-on-close job, then resume it. Descendants are
# owned even when the application exits before reporting readiness.
Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;
public static class PackagedSmokeJob {
  [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
  struct StartupInfo {
    public uint cb; public string reserved, desktop, title;
    public uint x, y, xSize, ySize, xChars, yChars, fill, flags;
    public ushort show, reservedSize; public IntPtr reservedBytes, input, output, error;
  }
  [StructLayout(LayoutKind.Sequential)]
  struct ProcessInfo { public IntPtr process, thread; public uint pid, tid; }
  [StructLayout(LayoutKind.Sequential)]
  struct BasicLimit {
    public long processTime, jobTime; public uint flags;
    public UIntPtr minWorkingSet, maxWorkingSet; public uint activeLimit;
    public UIntPtr affinity; public uint priority, scheduling;
  }
  [StructLayout(LayoutKind.Sequential)]
  struct ExtendedLimit {
    public BasicLimit basic;
    public ulong readOps, writeOps, otherOps, readBytes, writeBytes, otherBytes;
    public UIntPtr processMemory, jobMemory, peakProcessMemory, peakJobMemory;
  }
  [StructLayout(LayoutKind.Sequential)]
  struct Accounting {
    public long user, kernel, periodUser, periodKernel;
    public uint faults, totalProcesses, activeProcesses, terminatedProcesses;
  }
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern IntPtr CreateJobObjectW(IntPtr attributes, string name);
  [DllImport("kernel32.dll", SetLastError=true)]
  static extern bool SetInformationJobObject(IntPtr job, int kind, ref ExtendedLimit limit, uint size);
  [DllImport("kernel32.dll", SetLastError=true)]
  static extern bool QueryInformationJobObject(IntPtr job, int kind, out Accounting info, uint size, IntPtr length);
  [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  static extern bool CreateProcessW(string app, StringBuilder command, IntPtr processAttributes, IntPtr threadAttributes,
    bool inherit, uint flags, IntPtr environment, string directory, ref StartupInfo startup, out ProcessInfo process);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
  [DllImport("kernel32.dll", SetLastError=true)] static extern uint ResumeThread(IntPtr thread);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateJobObject(IntPtr job, uint code);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateProcess(IntPtr process, uint code);
  [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetExitCodeProcess(IntPtr process, out uint code);
  [DllImport("kernel32.dll", SetLastError=true)] static extern uint WaitForSingleObject(IntPtr handle, uint timeout);
  [DllImport("kernel32.dll")] static extern IntPtr GetStdHandle(int kind);
  [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
  static void Check(bool success, string operation) {
    if (!success) throw new Win32Exception(Marshal.GetLastWin32Error(), operation);
  }
  static string Quote(string value) {
    var text = new StringBuilder("\""); int slashes = 0;
    foreach (char character in value) {
      if (character == '\\') { slashes++; continue; }
      text.Append('\\', character == '"' ? slashes * 2 + 1 : slashes);
      text.Append(character); slashes = 0;
    }
    text.Append('\\', slashes * 2); text.Append('"'); return text.ToString();
  }
  public static int Run(string executable, string[] args, string record) {
    IntPtr job = CreateJobObjectW(IntPtr.Zero, null);
    Check(job != IntPtr.Zero, "Create smoke job");
    var process = new ProcessInfo(); bool assigned = false;
    try {
      var limit = new ExtendedLimit(); limit.basic.flags = 0x2000; // KILL_ON_JOB_CLOSE; no breakaway
      Check(SetInformationJobObject(job, 9, ref limit, (uint)Marshal.SizeOf(limit)), "Configure smoke job");
      var startup = new StartupInfo(); startup.cb = (uint)Marshal.SizeOf(startup);
      startup.flags = 0x100; startup.input = GetStdHandle(-10); startup.output = GetStdHandle(-11); startup.error = GetStdHandle(-12);
      var command = new StringBuilder(Quote(executable));
      foreach (var arg in args ?? Array.Empty<string>()) command.Append(' ').Append(Quote(arg));
      Check(CreateProcessW(executable, command, IntPtr.Zero, IntPtr.Zero, true, 4, IntPtr.Zero,
        Path.GetDirectoryName(executable), ref startup, out process), "Create suspended smoke application");
      Check(AssignProcessToJobObject(job, process.process), "Own smoke process tree"); assigned = true;
      File.WriteAllText(record, "{\"process_id\":" + process.pid + "}");
      Check(ResumeThread(process.thread) != 0xffffffff, "Resume smoke application");
      bool naturalExit = false;
      while (true) {
        uint waited = WaitForSingleObject(process.process, 20);
        if (waited == 0) { naturalExit = true; break; }
        if (File.Exists(record + ".stop")) break;
        Check(waited == 258, "Wait for smoke application");
      }
      uint code; Check(GetExitCodeProcess(process.process, out code), "Read smoke exit status");
      naturalExit = naturalExit || code != 259; // STILL_ACTIVE means the CLI stopped a live application.
      Check(TerminateJobObject(job, 1), "Reap surviving smoke descendants");
      var deadline = DateTime.UtcNow.AddSeconds(10);
      while (true) {
        Accounting info;
        Check(QueryInformationJobObject(job, 1, out info, (uint)Marshal.SizeOf(typeof(Accounting)), IntPtr.Zero), "Verify empty smoke job");
        if (info.activeProcesses == 0) break;
        if (DateTime.UtcNow >= deadline) throw new Exception("Smoke job descendants could not be reaped");
        System.Threading.Thread.Sleep(20);
      }
      File.WriteAllText(record + ".reaped", "{\"natural_exit\":" + (naturalExit ? "true" : "false") + ",\"exit_code\":" + code + "}");
      return unchecked((int)code);
    } finally {
      if (process.process != IntPtr.Zero && !assigned) TerminateProcess(process.process, 1);
      // Closing the job also contains exceptions and external launcher termination.
      CloseHandle(job);
      if (process.thread != IntPtr.Zero) CloseHandle(process.thread);
      if (process.process != IntPtr.Zero) CloseHandle(process.process);
    }
  }
}
'@
$command = Get-Content -Raw -LiteralPath $Configuration | ConvertFrom-Json
exit ([PackagedSmokeJob]::Run($command.executable, [string[]]$command.args, $command.record))
