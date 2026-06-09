import zlib
import re

file_path = "Eric Hobsbawm - Historia del Siglo XX.pdf"

with open(file_path, "rb") as f:
    data = f.read()

# Find all stream objects
streams = re.finditer(b"stream\r?\n(.*?)\r?\nendstream", data, re.DOTALL)

for match in streams:
    compressed_data = match.group(1)
    try:
        decompressed = zlib.decompress(compressed_data)
        if b"/Encoding" in decompressed:
            # Find the objects in the stream (PDF object streams start with pairs of numbers)
            # This is a bit complex to parse perfectly, so let's just find the dictionaries
            dicts = re.findall(b"<<.*?>>", decompressed, re.DOTALL)
            for d in dicts:
                if b"/Encoding" in d:
                    print(f"Object Dictionary: {d}")
    except:
        continue
