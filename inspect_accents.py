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
        # Search for p-e-r-[any]-o-d-o
        matches = re.finditer(b"per.odo", decompressed)
        for m in matches:
            chunk = decompressed[m.start():m.end()]
            print(f"Found 'período' variant: {chunk}")
            print(f"Bytes: {[b for b in chunk]}")
        
        # Search for i-n-v-e-s-t-i-g-a-c-i-[any]-n
        matches = re.finditer(b"investigaci.n", decompressed)
        for m in matches:
            chunk = decompressed[m.start():m.end()]
            print(f"Found 'investigación' variant: {chunk}")
            print(f"Bytes: {[b for b in chunk]}")
    except:
        continue
