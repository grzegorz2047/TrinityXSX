"""Resolve module+offset to function/file:line with Windows' dbghelp (reads the PDB next to the image).

usage: python -I pdbsym.py <image.exe|dll> <offset> [<offset> ...]
"""
import ctypes
import ctypes.wintypes as wt
import sys

dbghelp = ctypes.WinDLL("dbghelp.dll")
kernel32 = ctypes.WinDLL("kernel32.dll")


class SYMBOL_INFOW(ctypes.Structure):
    _fields_ = [
        ("SizeOfStruct", wt.ULONG), ("TypeIndex", wt.ULONG), ("Reserved", ctypes.c_uint64 * 2),
        ("Index", wt.ULONG), ("Size", wt.ULONG), ("ModBase", ctypes.c_uint64), ("Flags", wt.ULONG),
        ("Value", ctypes.c_uint64), ("Address", ctypes.c_uint64), ("Register", wt.ULONG),
        ("Scope", wt.ULONG), ("Tag", wt.ULONG), ("NameLen", wt.ULONG), ("MaxNameLen", wt.ULONG),
        ("Name", wt.WCHAR * 1024),
    ]


class IMAGEHLP_LINEW64(ctypes.Structure):
    _fields_ = [("SizeOfStruct", wt.DWORD), ("Key", ctypes.c_void_p), ("LineNumber", wt.DWORD),
                ("FileName", wt.LPWSTR), ("Address", ctypes.c_uint64)]


def main() -> None:
    image = sys.argv[1]
    # any unique value identifies the dbghelp session when the process is not invaded
    proc = wt.HANDLE(0x5EED)
    dbghelp.SymSetOptions(0x2 | 0x10)  # SYMOPT_UNDNAME | SYMOPT_LOAD_LINES
    dbghelp.SymInitializeW.argtypes = [wt.HANDLE, wt.LPCWSTR, wt.BOOL]
    dbghelp.SymFromAddrW.argtypes = [wt.HANDLE, ctypes.c_uint64, ctypes.POINTER(ctypes.c_uint64), ctypes.POINTER(SYMBOL_INFOW)]
    dbghelp.SymGetLineFromAddrW64.argtypes = [wt.HANDLE, ctypes.c_uint64, ctypes.POINTER(wt.DWORD), ctypes.POINTER(IMAGEHLP_LINEW64)]
    if not dbghelp.SymInitializeW(proc, None, False):
        sys.exit("SymInitialize failed")
    base = 0x10000000
    dbghelp.SymLoadModuleExW.restype = ctypes.c_uint64
    dbghelp.SymLoadModuleExW.argtypes = [wt.HANDLE, wt.HANDLE, wt.LPCWSTR, wt.LPCWSTR, ctypes.c_uint64, wt.DWORD, ctypes.c_void_p, wt.DWORD]
    got = dbghelp.SymLoadModuleExW(proc, None, image, None, base, 0, None, 0)
    if not got:
        sys.exit(f"SymLoadModuleEx failed: {ctypes.GetLastError()}")
    for off in sys.argv[2:]:
        addr = base + int(off, 16)
        sym = SYMBOL_INFOW()
        sym.SizeOfStruct = ctypes.sizeof(SYMBOL_INFOW) - ctypes.sizeof(wt.WCHAR * 1024) + ctypes.sizeof(wt.WCHAR)
        sym.MaxNameLen = 1024
        disp = ctypes.c_uint64()
        name = "?"
        if dbghelp.SymFromAddrW(proc, ctypes.c_uint64(addr), ctypes.byref(disp), ctypes.byref(sym)):
            name = f"{sym.Name}+{disp.value:#x}"
        line = IMAGEHLP_LINEW64()
        line.SizeOfStruct = ctypes.sizeof(IMAGEHLP_LINEW64)
        d32 = wt.DWORD()
        where = ""
        if dbghelp.SymGetLineFromAddrW64(proc, ctypes.c_uint64(addr), ctypes.byref(d32), ctypes.byref(line)):
            where = f"  {line.FileName}:{line.LineNumber}"
        print(f"{off}: {name}{where}")


if __name__ == "__main__":
    main()
