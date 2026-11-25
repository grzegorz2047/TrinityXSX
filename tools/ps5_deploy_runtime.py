"""Replace PS5 XCL using verified uploads and rename, preserving the previous binary."""

import argparse
import hashlib
import io
from datetime import datetime
from ftplib import FTP
from pathlib import Path


def deploy(host, port, binary, backup_dir):
    data = binary.read_bytes()
    digest = hashlib.sha256(data).hexdigest()
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S-%f")
    backup_dir.mkdir(parents=True, exist_ok=False)
    remote = "/data/prospero-win/prefix/drive_c/XCL/xcl.exe"
    pending = f"{remote}.pending-{stamp}"
    previous = f"{remote}.previous-{stamp}"
    with FTP() as ftp:
        ftp.connect(host, port, timeout=30)
        ftp.login()
        with (backup_dir / "xcl.previous.exe").open("xb") as stream:
            ftp.retrbinary(f"RETR {remote}", stream.write)
        ftp.storbinary(f"STOR {pending}", io.BytesIO(data))
        uploaded = hashlib.sha256()
        ftp.retrbinary(f"RETR {pending}", uploaded.update)
        if uploaded.hexdigest() != digest:
            raise RuntimeError(f"Upload checksum mismatch; original unchanged, pending: {pending}")
        ftp.rename(remote, previous)
        try:
            ftp.rename(pending, remote)
        except Exception:
            ftp.rename(previous, remote)
            raise
    (backup_dir / "deployment.txt").write_text(
        f"source={binary.resolve()}\nsha256={digest}\nremote={remote}\nprevious={previous}\n",
        encoding="utf-8",
    )
    print(f"Deployed {binary.name}: SHA256 {digest}")
    print(f"Backup: {backup_dir}; PS5: {previous}")
    print("Restart Minecraft in prospero-win to load the replacement.")


if __name__ == "__main__":
    root = Path(__file__).resolve().parent.parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("host")
    parser.add_argument("--port", type=int, default=2121)
    parser.add_argument("--binary", type=Path, default=root / "dist/prospero-win/prefix/drive_c/XCL/xcl.exe")
    args = parser.parse_args()
    backup = root / "ps5-logs/minecraft" / datetime.now().strftime("deploy-%Y%m%d-%H%M%S-%f")
    deploy(args.host, args.port, args.binary, backup)
