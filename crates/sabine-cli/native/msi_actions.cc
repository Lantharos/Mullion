#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <msiquery.h>

#include <array>
#include <stdexcept>
#include <string>
#include <vector>

namespace {
class Handle {
 public:
  explicit Handle(HANDLE value = nullptr) : value_(value) {}
  ~Handle() { Reset(); }
  Handle(const Handle&) = delete;
  Handle& operator=(const Handle&) = delete;
  HANDLE Get() const { return value_; }
  void Reset() {
    if (value_ && value_ != INVALID_HANDLE_VALUE) CloseHandle(value_);
    value_ = nullptr;
  }
 private:
  HANDLE value_;
};

class ChildProcess {
 public:
  explicit ChildProcess(HANDLE process) : process_(process) {}
  ~ChildProcess() {
    if (WaitForSingleObject(process_.Get(), 0) == WAIT_TIMEOUT) {
      TerminateProcess(process_.Get(), ERROR_INSTALL_FAILURE);
      WaitForSingleObject(process_.Get(), 5000);
    }
  }
  HANDLE Get() const { return process_.Get(); }
 private:
  Handle process_;
};

std::wstring Wide(const std::string& value) {
  const int count = MultiByteToWideChar(CP_UTF8, 0, value.data(), static_cast<int>(value.size()), nullptr, 0);
  std::wstring result(count, L'\0');
  MultiByteToWideChar(CP_UTF8, 0, value.data(), static_cast<int>(value.size()), result.data(), count);
  return result;
}

void Check(bool success, const char* operation) {
  if (!success) throw std::runtime_error(std::string(operation) + " (Windows error " + std::to_string(GetLastError()) + ")");
}

bool Log(MSIHANDLE install, const std::string& message) {
  PMSIHANDLE record = MsiCreateRecord(1);
  MsiRecordSetStringW(record, 0, L"Sabine: [1]");
  MsiRecordSetStringW(record, 1, Wide(message).c_str());
  const bool cancelled = MsiProcessMessage(install, INSTALLMESSAGE_INFO, record) == IDCANCEL;
  return (MsiProcessMessage(install, INSTALLMESSAGE_ACTIONDATA, record) == IDCANCEL) || cancelled;
}

class CancellationFile {
 public:
  CancellationFile() {
    std::array<wchar_t, MAX_PATH> directory{};
    const DWORD length = GetTempPathW(static_cast<DWORD>(directory.size()), directory.data());
    Check(length > 0 && length < directory.size(), "Could not find setup temporary directory");
    Check(GetTempFileNameW(directory.data(), L"sbn", 0, path_.data()) != 0, "Could not reserve setup cancellation file");
    Check(DeleteFileW(path_.data()) != FALSE, "Could not prepare setup cancellation file");
  }
  ~CancellationFile() { DeleteFileW(path_.data()); }
  const wchar_t* Path() const { return path_.data(); }
  void Request() {
    Handle file(CreateFileW(Path(), GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_DELETE,
        nullptr, CREATE_ALWAYS, FILE_ATTRIBUTE_TEMPORARY, nullptr));
    Check(file.Get() != INVALID_HANDLE_VALUE, "Could not request setup cancellation");
  }
 private:
  std::array<wchar_t, MAX_PATH> path_{};
};

std::wstring CommandLine(MSIHANDLE install) {
  wchar_t empty = L'\0';
  DWORD length = 0;
  const UINT result = MsiGetPropertyW(install, L"CustomActionData", &empty, &length);
  if (result != ERROR_MORE_DATA || length == 0 || length > 32000) {
    throw std::runtime_error("Setup command is missing or too long");
  }
  std::wstring command(length + 1, L'\0');
  const UINT read_result = MsiGetPropertyW(install, L"CustomActionData", command.data(), &length);
  if (read_result != ERROR_SUCCESS) {
    throw std::runtime_error("Could not read setup command (MSI error " + std::to_string(read_result) + ")");
  }
  command.resize(length);
  return command;
}

bool DrainOutput(MSIHANDLE install, HANDLE pipe, std::string& pending) {
  bool cancelled = false;
  std::array<char, 8192> buffer{};
  DWORD available = 0;
  for (int chunk = 0; chunk < 8 && PeekNamedPipe(pipe, nullptr, 0, nullptr, &available, nullptr) && available; ++chunk) {
    DWORD read = 0;
    if (!ReadFile(pipe, buffer.data(), static_cast<DWORD>(buffer.size()), &read, nullptr) || !read) break;
    pending.append(buffer.data(), read);
    size_t start = 0;
    for (size_t end = pending.find('\n'); end != std::string::npos; end = pending.find('\n', start)) {
      cancelled |= Log(install, pending.substr(start, end - start));
      start = end + 1;
    }
    pending.erase(0, start);
    if (pending.size() >= 65536) {
      cancelled |= Log(install, pending);
      pending.clear();
    }
  }
  return cancelled;
}

UINT Run(MSIHANDLE install) {
  CancellationFile cancellation;
  std::wstring command = CommandLine(install) + L" --sabine-install-cancel \"" + cancellation.Path() + L"\"";
  SECURITY_ATTRIBUTES security{sizeof(SECURITY_ATTRIBUTES), nullptr, TRUE};
  HANDLE read = nullptr;
  HANDLE write = nullptr;
  Check(CreatePipe(&read, &write, &security, 0) != FALSE, "Could not create setup output pipe");
  Handle output(read);
  Handle writer(write);
  Check(SetHandleInformation(read, HANDLE_FLAG_INHERIT, 0) != FALSE, "Could not protect setup output handle");
  Handle input(CreateFileW(L"NUL", GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE, &security, OPEN_EXISTING, 0, nullptr));
  Check(input.Get() != INVALID_HANDLE_VALUE, "Could not open setup input");
  SIZE_T attributes_size = 0;
  InitializeProcThreadAttributeList(nullptr, 1, 0, &attributes_size);
  std::vector<unsigned char> attributes(attributes_size);
  auto* list = reinterpret_cast<LPPROC_THREAD_ATTRIBUTE_LIST>(attributes.data());
  Check(InitializeProcThreadAttributeList(list, 1, 0, &attributes_size) != FALSE, "Could not create setup process attributes");
  struct AttributeCleanup {
    LPPROC_THREAD_ATTRIBUTE_LIST list;
    ~AttributeCleanup() { DeleteProcThreadAttributeList(list); }
  } cleanup{list};
  HANDLE inherited[] = {writer.Get(), input.Get()};
  Check(UpdateProcThreadAttribute(list, 0, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, inherited, sizeof(inherited), nullptr, nullptr) != FALSE, "Could not restrict setup handle inheritance");
  STARTUPINFOEXW startup{};
  startup.StartupInfo.cb = sizeof(startup);
  startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
  startup.StartupInfo.hStdOutput = startup.StartupInfo.hStdError = writer.Get();
  startup.StartupInfo.hStdInput = input.Get();
  startup.lpAttributeList = list;
  PROCESS_INFORMATION info{};
  Check(CreateProcessW(nullptr, command.data(), nullptr, nullptr, TRUE, CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
      nullptr, nullptr, &startup.StartupInfo, &info) != FALSE, "Could not start Sabine setup");
  ChildProcess process(info.hProcess);
  Handle thread(info.hThread);
  writer.Reset();
  input.Reset();
  PMSIHANDLE progress = MsiCreateRecord(2);
  MsiRecordSetInteger(progress, 1, 2);
  MsiRecordSetInteger(progress, 2, 0);
  const bool rollback = MsiGetMode(install, MSIRUNMODE_ROLLBACK) != FALSE;
  if (!rollback) {
    PMSIHANDLE controls = MsiCreateRecord(2);
    MsiRecordSetInteger(controls, 1, 2);
    MsiRecordSetInteger(controls, 2, 1);
    MsiProcessMessage(install, INSTALLMESSAGE_COMMONDATA, controls);
  }
  const ULONGLONG started = GetTickCount64();
  ULONGLONG cancelled_at = 0;
  std::string pending;
  while (WaitForSingleObject(process.Get(), 100) == WAIT_TIMEOUT) {
    const bool requested = DrainOutput(install, output.Get(), pending) |
        (MsiProcessMessage(install, INSTALLMESSAGE_PROGRESS, progress) == IDCANCEL);
    if (requested && !rollback && !cancelled_at) {
      cancellation.Request();
      cancelled_at = GetTickCount64();
      Log(install, "Cancelling setup...");
    }
    const ULONGLONG now = GetTickCount64();
    if ((cancelled_at && now - cancelled_at >= 30000) || now - started >= 60 * 60 * 1000) {
      Log(install, cancelled_at ? "Stopping cancelled setup" : "Setup exceeded its one-hour deadline");
      TerminateProcess(process.Get(), cancelled_at ? ERROR_INSTALL_USEREXIT : ERROR_INSTALL_FAILURE);
      WaitForSingleObject(process.Get(), 5000);
      return cancelled_at ? ERROR_INSTALL_USEREXIT : ERROR_INSTALL_FAILURE;
    }
  }
  if (DrainOutput(install, output.Get(), pending) && !rollback) cancelled_at = GetTickCount64();
  if (!pending.empty() && Log(install, pending) && !rollback) cancelled_at = GetTickCount64();
  DWORD exit_code = ERROR_INSTALL_FAILURE;
  Check(GetExitCodeProcess(process.Get(), &exit_code) != FALSE, "Could not read setup result");
  if (cancelled_at || exit_code == ERROR_INSTALL_USEREXIT) return ERROR_INSTALL_USEREXIT;
  return exit_code == 0 ? ERROR_SUCCESS : ERROR_INSTALL_FAILURE;
}
}

extern "C" __declspec(dllexport) UINT __stdcall SabineSetup(MSIHANDLE install) {
  try {
    return Run(install);
  } catch (const std::exception& error) {
    Log(install, error.what());
    return ERROR_INSTALL_FAILURE;
  }
}
