import fitz

doc = fitz.open("Eric Hobsbawm - Historia del Siglo XX.pdf")

for i in range(10):
    page = doc[i]
    try:
        content = page.read_contents()
        if content:
            with open(f"page_{i}.txt", "wb") as f:
                f.write(content)
    except:
        pass
print("Done dumping pages")