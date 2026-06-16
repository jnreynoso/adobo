# Script de Instalación de Adobo Reader para Windows
# --------------------------------------------------

$ErrorActionPreference = "Stop"
Clear-Host

Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host "             Instalador Oficial de Adobo Reader           " -ForegroundColor Cyan
Write-Host "==========================================================" -ForegroundColor Cyan
Write-Host ""

# 1. Compilación en modo Release
Write-Host "[1/6] Compilando Adobo Reader en modo Release..." -ForegroundColor Yellow
try {
    # Ejecutamos cargo build
    $cargoProcess = Start-Process cargo -ArgumentList "build --release --bin ufreader" -NoNewWindow -PassThru -Wait
    if ($cargoProcess.ExitCode -ne 0) {
        throw "La compilación mediante Cargo falló."
    }
    Write-Host " -> Compilación completada con éxito." -ForegroundColor Green
} catch {
    Write-Error "Error de compilación: $_"
    Exit 1
}

# 2. Creación del directorio de instalación
$InstallDir = "$env:LOCALAPPDATA\Adobo"
Write-Host "[2/6] Creando el directorio de instalación en: $InstallDir" -ForegroundColor Yellow
if (!(Test-Path $InstallDir)) {
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
}
Write-Host " -> Directorio creado/verificado." -ForegroundColor Green

# 3. Copiado de archivos ejecutables y assets
Write-Host "[3/6] Copiando binarios y recursos..." -ForegroundColor Yellow
try {
    # Copiar ejecutable principal renombrado a adobo.exe
    Copy-Item -Path "target\release\ufreader.exe" -Destination "$InstallDir\adobo.exe" -Force
    
    # Copiar directorio de assets
    if (Test-Path "assets") {
        Copy-Item -Path "assets" -Destination $InstallDir -Recurse -Force
    }
    Write-Host " -> Archivos copiados correctamente." -ForegroundColor Green
} catch {
    Write-Error "Error al copiar archivos: $_"
    Exit 1
}

# 4. Creación de accesos directos
Write-Host "[4/6] Creando accesos directos..." -ForegroundColor Yellow
try {
    $WshShell = New-Object -ComObject WScript.Shell
    
    # Acceso directo en el Escritorio
    $DesktopShortcutPath = "$env:USERPROFILE\Desktop\Adobo Reader.lnk"
    $Shortcut = $WshShell.CreateShortcut($DesktopShortcutPath)
    $Shortcut.TargetPath = "$InstallDir\adobo.exe"
    $Shortcut.WorkingDirectory = $InstallDir
    $Shortcut.Description = "Lector de PDF Adobo Reader"
    $Shortcut.IconLocation = "$InstallDir\assets\logo.ico"
    $Shortcut.Save()
    
    # Acceso directo en el Menú de Inicio
    $StartMenuPath = "$env:APPDATA\Microsoft\Windows\Start Menu\Programs"
    if (!(Test-Path $StartMenuPath)) {
        New-Item -ItemType Directory -Force -Path $StartMenuPath | Out-Null
    }
    $SMShortcutPath = "$StartMenuPath\Adobo Reader.lnk"
    $ShortcutSM = $WshShell.CreateShortcut($SMShortcutPath)
    $ShortcutSM.TargetPath = "$InstallDir\adobo.exe"
    $ShortcutSM.WorkingDirectory = $InstallDir
    $ShortcutSM.Description = "Lector de PDF Adobo Reader"
    $ShortcutSM.IconLocation = "$InstallDir\assets\logo.ico"
    $ShortcutSM.Save()
    
    Write-Host " -> Accesos directos creados en el Escritorio y Menú Inicio." -ForegroundColor Green
} catch {
    Write-Warning "No se pudieron crear algunos accesos directos: $_"
}

# 5. Registro de la asociación del tipo de archivo (.pdf)
Write-Host "[5/6] Registrando la asociación de archivos .pdf..." -ForegroundColor Yellow
try {
    # Claves en HKCU (no requiere elevación de administrador)
    $ClassKey = "HKCU:\Software\Classes\AdoboReader.pdf"
    if (!(Test-Path $ClassKey)) {
        New-Item -Path $ClassKey -Force | Out-Null
    }
    Set-ItemProperty -Path $ClassKey -Name "(Default)" -Value "Documento PDF (Adobo Reader)" -Force
    
    # Comando de apertura
    $CommandKey = "$ClassKey\shell\open\command"
    if (!(Test-Path $CommandKey)) {
        New-Item -Path $CommandKey -Force | Out-Null
    }
    Set-ItemProperty -Path $CommandKey -Name "(Default)" -Value "`"$InstallDir\adobo.exe`" `"%1`"" -Force
    
    # Icono predeterminado
    $IconKey = "$ClassKey\DefaultIcon"
    if (!(Test-Path $IconKey)) {
        New-Item -Path $IconKey -Force | Out-Null
    }
    Set-ItemProperty -Path $IconKey -Name "(Default)" -Value "`"$InstallDir\assets\logo.ico`"" -Force

    # Asignar la extensión .pdf a la clase registrada
    $PdfExtKey = "HKCU:\Software\Classes\.pdf"
    if (!(Test-Path $PdfExtKey)) {
        New-Item -Path $PdfExtKey -Force | Out-Null
    }
    Set-ItemProperty -Path $PdfExtKey -Name "(Default)" -Value "AdoboReader.pdf" -Force

    # Notificar al shell de Windows sobre el cambio de asociación de archivos para refrescar los iconos
    $code = @'
    [System.Runtime.InteropServices.DllImport("shell32.dll", CharSet=System.Runtime.InteropServices.CharSet.Auto, SetLastError=true)]
    public static extern void SHChangeNotify(int wEventId, int uFlags, System.IntPtr dwItem1, System.IntPtr dwItem2);
'@
    Add-Type -MemberDefinition $code -Namespace Shell32 -Name NativeMethods
    [Shell32.NativeMethods]::SHChangeNotify(0x08000000, 0, [System.IntPtr]::Zero, [System.IntPtr]::Zero) # SHCNE_ASSOCCHANGED
    
    Write-Host " -> Asociación de archivos .pdf registrada en el registro de Windows de forma exitosa." -ForegroundColor Green
} catch {
    Write-Warning "No se pudo registrar la asociación de archivos .pdf: $_"
}

# 6. Registro del directorio en el PATH del usuario
Write-Host "[6/6] Añadiendo directorio al PATH de usuario..." -ForegroundColor Yellow
try {
    $UserPath = [Environment]::GetEnvironmentVariable("PATH", "User")
    if ($UserPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("PATH", "$UserPath;$InstallDir", "User")
        Write-Host " -> Directorio añadido al PATH de usuario." -ForegroundColor Green
    } else {
        Write-Host " -> El directorio ya se encuentra registrado en el PATH." -ForegroundColor Green
    }
} catch {
    Write-Warning "No se pudo añadir el directorio al PATH: $_"
}

Write-Host ""
Write-Host "==========================================================" -ForegroundColor Green
Write-Host "      ¡Instalación de Adobo Reader Completada con Éxito!  " -ForegroundColor Green
Write-Host "==========================================================" -ForegroundColor Green
Write-Host " Puedes iniciar Adobo Reader desde el Escritorio, el menú  " -ForegroundColor White
Write-Host " de inicio o simplemente escribiendo 'adobo' en la consola." -ForegroundColor White
Write-Host "==========================================================" -ForegroundColor Green
Write-Host ""
