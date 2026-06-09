import fitz
import sys

doc = fitz.open("Eric Hobsbawm - Historia del Siglo XX.pdf")
page = doc[4] # Page 5 (0-indexed)

print("--- Fonts on Page 5 ---")
fonts = page.get_fonts()
for font in fonts:
    print(font)

print("\n--- Raw Content Stream ---")
# Get raw commands
try:
    contents = page.read_contents()
    # Decode if needed
    print(contents.decode('latin1')[:2000])
except Exception as e:
    print(e)

print("\n--- Extracted Text Dict ---")
blocks = page.get_text("dict")["blocks"]
for b in blocks[:5]:
    if "lines" in b:
        for l in b["lines"]:
            for s in l["spans"]:
                print(f"Font: '{s['font']}', Size: {s['size']}, Text: '{s['text']}'")
