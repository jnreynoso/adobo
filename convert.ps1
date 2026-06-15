Add-Type -AssemblyName System.Drawing
$img = [System.Drawing.Image]::FromFile('assets\logo.png')
$img.Save('assets\logo_real.png', [System.Drawing.Imaging.ImageFormat]::Png)
$img.Dispose()
Move-Item -Force 'assets\logo_real.png' 'assets\logo.png'
