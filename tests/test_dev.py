"""Host-only checks for flashing safeguards and the actual LAN upload client."""
import hashlib
import hmac
from http.server import BaseHTTPRequestHandler, HTTPServer
from importlib.machinery import SourceFileLoader
from pathlib import Path
import tempfile
import threading
import unittest
from unittest.mock import patch

dev = SourceFileLoader("bluetemp_dev", str(Path(__file__).resolve().parents[1] / "scripts/dev")).load_module()


def app_image():
    image = bytearray(288)
    image[0:2] = b"\xe9\x05"
    image[23] = 1
    image[32:36] = bytes.fromhex("3254cdab")
    image[80:89] = b"bluetemp\0"
    return bytes(image) + hashlib.sha256(image).digest()


class DevTests(unittest.TestCase):
    def test_board_revision_flash_size_and_wrong_chip(self):
        dev.verify_board("Chip type:         ESP32 (revision v3.1)\nFlash size:        4MB\n")
        for output in ["", "ESP32 (revision v1.0)\nFlash size: 4MB", "ESP32-S3 (revision v3.0)\nFlash size: 8MB",
                       "ESP32 (revision v3.0)\nFlash size: 2MB", "ESP32 (revision v3.0)\nFlash size: unknown"]:
            with self.assertRaises(ValueError):
                dev.verify_board(output)

    def test_image_validation(self):
        image = app_image()
        dev.validate_image(image)
        for index in [0, 1, 12, 23, 32, 80, 100, len(image) - 1]:
            corrupt = bytearray(image)
            corrupt[index] ^= 0xff
            with self.assertRaises(ValueError):
                dev.validate_image(corrupt)
        with self.assertRaises(ValueError):
            dev.validate_image(image[:287])

    def test_real_http_upload_signature_and_content_length(self):
        received = {}

        class Handler(BaseHTTPRequestHandler):
            def do_POST(self):
                received["path"] = self.path
                received["headers"] = self.headers
                received["body"] = self.rfile.read(int(self.headers["Content-Length"]))
                self.send_response(200)
                self.send_header("Content-Length", "2")
                self.end_headers()
                self.wfile.write(b"OK")

            def log_message(self, *_args):
                pass

        with tempfile.TemporaryDirectory() as directory:
            image_path = Path(directory) / "image.bin"
            image_path.write_bytes(app_image())
            with HTTPServer(("127.0.0.1", 0), Handler) as server:
                thread = threading.Thread(target=server.handle_request)
                thread.start()
                try:
                    with patch.object(dev, "build"), patch.object(dev, "IMAGE", image_path), patch.object(dev, "key_bytes", return_value=bytes(range(32))):
                        dev.ota(f"http://127.0.0.1:{server.server_port}")
                finally:
                    thread.join(timeout=5)
        self.assertEqual(received["path"], "/ota")
        self.assertEqual(received["body"], app_image())
        digest = hashlib.sha256(app_image()).digest()
        manifest = b"bluetemp-ota-v1\0" + len(app_image()).to_bytes(4, "big") + digest
        expected = hmac.new(bytes(range(32)), manifest, hashlib.sha256).hexdigest()
        self.assertEqual(received["headers"]["X-Image-Signature"], expected)
        self.assertEqual(received["headers"]["X-Image-SHA256"], digest.hex())


if __name__ == "__main__":
    unittest.main()
