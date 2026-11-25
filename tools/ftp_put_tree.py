"""Upload a local directory tree to an FTP server in one session (the PS5's FTP has no NLST/EPSV).

usage: python -I ftp_put_tree.py <host> <port> <local dir> <remote dir>
Existing remote files with the same name are overwritten; nothing is deleted.
"""
import ftplib
import os
import sys


def main() -> None:
    host, port, local, remote = sys.argv[1], int(sys.argv[2]), sys.argv[3], sys.argv[4].rstrip("/")
    ftp = ftplib.FTP()
    ftp.connect(host, port, timeout=60)
    ftp.login()
    ftp.set_pasv(True)
    made = set()

    def mkdirs(path: str) -> None:
        parts = path.strip("/").split("/")
        for i in range(1, len(parts) + 1):
            p = "/" + "/".join(parts[:i])
            if p in made:
                continue
            try:
                ftp.mkd(p)
            except ftplib.error_perm:
                pass  # exists
            made.add(p)

    files = bytes_sent = 0
    for dirpath, _dirs, names in os.walk(local):
        rel = os.path.relpath(dirpath, local).replace("\\", "/")
        rdir = remote if rel == "." else f"{remote}/{rel}"
        mkdirs(rdir)
        for n in names:
            src = os.path.join(dirpath, n)
            with open(src, "rb") as f:
                try:
                    ftp.storbinary(f"STOR {rdir}/{n}", f, blocksize=1 << 20)
                except ftplib.error_perm as e:
                    sys.exit(f"STOR {rdir}/{n}: {e}")
            files += 1
            bytes_sent += os.path.getsize(src)
            if files % 200 == 0:
                print(f"{files} files, {bytes_sent >> 20} MB", flush=True)
    ftp.quit()
    print(f"done: {files} files, {bytes_sent >> 20} MB")


if __name__ == "__main__":
    main()
