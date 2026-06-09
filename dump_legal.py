import zlib
import re

file_path = "Eric Hobsbawm - Historia del Siglo XX.pdf"

with open(file_path, "rb") as f:
    data = f.read()

streams = re.finditer(b"stream\r?\n(.*?)\r?\nendstream", data, re.DOTALL)

for match in streams:
    compressed_data = match.group(1)
    try:
        decompressed = zlib.decompress(compressed_data)
        if b"fotocopia" in decompressed:
            print("--- Found Legal Text Match ---")
            idx = decompressed.find(b"fotocopia")
            start = max(0, idx - 200)
            end = min(len(decompressed), idx + 200)
            print(decompressed[start:end].decode('latin1'))
    except Exception as e:
        continue
