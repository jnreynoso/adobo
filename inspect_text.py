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
        # Search for fragments of words likely to have accents
        # "p gina" (página)
        if b"p" in decompressed and b"gina" in decompressed:
            # Let's find "p" followed by one byte and then "gina"
            idx = decompressed.find(b"gina")
            if idx > 0:
                print(f"Potential 'página' match: {decompressed[idx-5:idx+5]}")
                print(f"Bytes: {[b for b in decompressed[idx-5:idx+5]]}")
        if b"XX" in decompressed:
             print(f"Context around XX: {decompressed[decompressed.find(b'XX')-20:decompressed.find(b'XX')+20]}")
    except:
        continue
