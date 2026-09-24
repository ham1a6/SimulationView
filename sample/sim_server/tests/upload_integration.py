"""実サーバーで受信・上限・切断時の後始末を確認する（外部依存なし）。"""
import http.client
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time


def main():
    executable = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix="sim3dview-upload-") as folder:
        root = Path(folder)
        process = subprocess.Popen(
            [executable, "0", "--host", "127.0.0.1", "--upload-dir", str(root / "uploads")],
            cwd=root, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
            encoding="utf-8", errors="replace",
        )
        try:
            # サーバーの起動通知までを専用スレッドで読み、起動失敗をタイムアウトで検出する。
            import queue
            import threading
            lines = queue.Queue()
            def read_lines():
                for line in process.stdout:
                    lines.put(line)
                lines.put(None)
            threading.Thread(target=read_lines, daemon=True).start()
            while True:
                line = lines.get(timeout=20)
                if line is None:
                    raise RuntimeError("起動通知前にサーバーが終了しました")
                if line.startswith("SIM3DVIEW_READY "):
                    port = int(line.split()[1])
                    break

            def request(method, body=None, headers=None, chunked=False):
                connection = http.client.HTTPConnection("127.0.0.1", port, timeout=15)
                connection.request(method, "/uploads", body, headers or {}, encode_chunked=chunked)
                response = connection.getresponse()
                result = response.status, response.read().decode(), dict(response.getheaders())
                connection.close()
                return result

            headers = {"Content-Type": "application/octet-stream", "Origin": "http://localhost:8081"}
            status, _, cors = request("OPTIONS", headers={"Origin": headers["Origin"], "Access-Control-Request-Method": "POST", "Access-Control-Request-Headers": "content-type"})
            assert status == 204 and cors["Access-Control-Allow-Origin"] == "*"
            assert cors["Access-Control-Allow-Methods"] == "POST"
            payload = bytes(range(256)) * 4096
            saved = []
            for body in [payload, payload, b""]:
                status, identifier, _ = request("POST", body, headers)
                assert status == 201, status
                destination = (root / "uploads" / identifier).resolve()
                assert destination.is_relative_to(root / "uploads")
                assert destination.read_bytes() == body
                saved.append(identifier)
            assert len(set(saved)) == 3
            status, identifier, _ = request("POST", iter([b"abc", b"\x00\xff"]), headers, True)
            assert status == 201
            assert (root / "uploads" / identifier).read_bytes() == b"abc\x00\xff"
            assert request("POST", b"bad", {"Content-Type": "text/plain"})[0] == 415
            # 境界ちょうどは成功し、1バイト超過は失敗する。
            block = b"x" * (1024 * 1024)
            status, identifier, _ = request("POST", (block for _ in range(64)), headers, True)
            assert status == 201
            assert (root / "uploads" / identifier).stat().st_size == 64 * len(block)
            before = set((root / "uploads").iterdir())
            assert request("POST", iter([block] * 64 + [b"x"]), headers, True)[0] == 413
            assert set((root / "uploads").iterdir()) == before
            # Content-Length分を送り切らずに切断する。
            with socket.create_connection(("127.0.0.1", port), timeout=5) as connection:
                connection.sendall(b"POST /uploads HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/octet-stream\r\nContent-Length: 1000000\r\n\r\npartial")
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline:
                request("OPTIONS")
                if set((root / "uploads").iterdir()) == before:
                    break
                time.sleep(0.02)
            assert set((root / "uploads").iterdir()) == before
            assert not list(root.rglob("*.part"))
            print("OK: CORS、バイト一致、空ファイル、再送、chunked、64 MiB境界、415、413、切断時の削除")
        finally:
            process.terminate()
            process.wait(timeout=10)
            process.stdout.close()


if __name__ == "__main__":
    main()
