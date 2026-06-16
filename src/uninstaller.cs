using System;
using System.IO;
using System.Windows.Forms;
using Microsoft.Win32;
using System.Runtime.InteropServices;

namespace AdoboUninstaller {
    public class Uninstaller {
        [DllImport("shell32.dll", CharSet = CharSet.Auto, SetLastError = true)]
        public static extern void SHChangeNotify(int wEventId, int uFlags, IntPtr dwItem1, IntPtr dwItem2);

        [STAThread]
        public static void Main() {
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);

            DialogResult result = MessageBox.Show(
                "¿Está seguro de que desea desinstalar Adobo Reader por completo de su equipo?",
                "Desinstalar Adobo Reader",
                MessageBoxButtons.YesNo,
                MessageBoxIcon.Question
            );

            if (result == DialogResult.No) {
                return;
            }

            string installPath = null;
            try {
                using (RegistryKey key = Registry.LocalMachine.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Uninstall\AdoboReader")) {
                    if (key != null) {
                        installPath = key.GetValue("InstallLocation") as string;
                    }
                }
            } catch {}

            if (string.IsNullOrEmpty(installPath)) {
                installPath = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "Adobo");
            }

            try {
                // 1. Eliminar Accesos Directos
                string desktopLnk = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Desktop), "Adobo Reader.lnk");
                if (File.Exists(desktopLnk)) {
                    File.Delete(desktopLnk);
                }

                string startMenuLnk = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Programs), "Adobo Reader.lnk");
                if (File.Exists(startMenuLnk)) {
                    File.Delete(startMenuLnk);
                }

                // 2. Eliminar Asociaciones del Registro
                using (RegistryKey pdfKey = Registry.LocalMachine.OpenSubKey(@"Software\Classes\.pdf", true)) {
                    if (pdfKey != null && "AdoboReader.pdf".Equals(pdfKey.GetValue(""))) {
                        pdfKey.SetValue("", "");
                    }
                }
                
                try {
                    Registry.LocalMachine.DeleteSubKeyTree(@"Software\Classes\AdoboReader.pdf", false);
                } catch {}

                // Forzar refresco del Explorador de Windows
                SHChangeNotify(0x08000000, 0, IntPtr.Zero, IntPtr.Zero);

                // 3. Eliminar del PATH
                string pathVar = Environment.GetEnvironmentVariable("PATH", EnvironmentVariableTarget.User);
                if (pathVar != null && pathVar.Contains(installPath)) {
                    string cleanPath = pathVar.Replace(installPath + ";", "").Replace(";" + installPath, "").Replace(installPath, "");
                    Environment.SetEnvironmentVariable("PATH", cleanPath, EnvironmentVariableTarget.User);
                }

                // 4. Eliminar Clave de Desinstalación
                try {
                    Registry.LocalMachine.DeleteSubKeyTree(@"Software\Microsoft\Windows\CurrentVersion\Uninstall\AdoboReader", false);
                } catch {}

                // 5. Crear script de limpieza temporal para auto-eliminación
                string tempBat = Path.Combine(Path.GetTempPath(), "cleanup_adobo.bat");
                using (StreamWriter sw = new StreamWriter(tempBat)) {
                    sw.WriteLine("@echo off");
                    sw.WriteLine(":loop");
                    sw.WriteLine("taskkill /f /im adobo.exe >nul 2>&1");
                    sw.WriteLine("del \"" + Path.Combine(installPath, "adobo.exe") + "\" >nul 2>&1");
                    sw.WriteLine("del \"" + Path.Combine(installPath, "desinstalar.exe") + "\" >nul 2>&1");
                    sw.WriteLine("if exist \"" + Path.Combine(installPath, "desinstalar.exe") + "\" goto loop");
                    sw.WriteLine("rd /s /q \"" + installPath + "\" >nul 2>&1");
                    sw.WriteLine("del \"%~f0\" >nul 2>&1");
                }

                // Ejecutar script bat de forma oculta
                System.Diagnostics.ProcessStartInfo psi = new System.Diagnostics.ProcessStartInfo(tempBat);
                psi.WindowStyle = System.Diagnostics.ProcessWindowStyle.Hidden;
                psi.CreateNoWindow = true;
                psi.UseShellExecute = true;
                System.Diagnostics.Process.Start(psi);

                MessageBox.Show("Adobo Reader se ha desinstalado correctamente de su equipo.", "Desinstalación Completada", MessageBoxButtons.OK, MessageBoxIcon.Information);
            }
            catch (Exception ex) {
                MessageBox.Show("Ocurrió un error durante la desinstalación: " + ex.Message, "Error de Desinstalación", MessageBoxButtons.OK, MessageBoxIcon.Error);
            }
        }
    }
}
