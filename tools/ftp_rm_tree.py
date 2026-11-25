"""Empty a folder on an FTP server (its files and subfolders; the folder itself stays).

usage: python -I ftp_rm_tree.py <host> <port> <remote dir>
e.g. the test title's XCL overlay on the PS5, which a second run would otherwise see:
  MSYS_NO_PATHCONV=1 python -I tools/ftp_rm_tree.py <ps5> 2121 \
      /data/prospero-win/prefix/drive_c/users/prospero/AppData/Local/XCL/XCL.TestTitle/overlay
The PS5's FTP server answers DELE with 226, which ftplib's delete() takes for an error: commands
are sent as they are and any 2xx reply is success.
"""
import ftplib
import sys


def main() -> None:
    host, port, root = sys.argv[1], int(sys.argv[2]), sys.argv[3].rstrip("/")
    if root.count("/") < 3:
        sys.exit(f"refusing to empty {root!r}: give a folder at least three levels deep")
    ftp = ftplib.FTP()
    ftp.connect(host, port, timeout=30)
    ftp.login()
    ftp.set_pasv(True)

    def cmd(c: str) -> None:
        try:
            r = ftp.sendcmd(c)
        except ftplib.error_reply as e:
            r = str(e)
        if not r.startswith("2"):
            sys.exit(f"{c}: {r}")

    def entries(p: str):
        lines: list[str] = []
        ftp.retrlines(f"LIST {p}", lines.append)
        out = []
        for line in lines:
            name = line.split(None, 8)[-1]
            if name not in (".", ".."):
                out.append((line.startswith("d"), name))
        return out

    def empty(p: str) -> None:
        for is_dir, name in entries(p):
            q = f"{p}/{name}"
            if is_dir:
                empty(q)
                cmd(f"RMD {q}")
            else:
                cmd(f"DELE {q}")
            print(("rmdir " if is_dir else "del ") + q[len(root):])

    empty(root)
    ftp.quit()


if __name__ == "__main__":
    main()
