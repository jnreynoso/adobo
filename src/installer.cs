using System;
using System.IO;
using System.Drawing;
using System.Reflection;
using System.Windows.Forms;
using Microsoft.Win32;
using System.Runtime.InteropServices;

namespace AdoboInstaller {
    public class InstallerForm : Form {
        private Panel bannerPanel;
        private Label bannerLabel;
        private Label contentLabel;
        private Button btnNext;
        private Button btnBack;
        private Button btnCancel;
        private ProgressBar progressBar;
        private TextBox txtInstallPath;
        private Button btnBrowse;
        
        private CheckBox chkDesktop;
        private CheckBox chkStartMenu;
        private CheckBox chkAssociate;
        private CheckBox chkPath;
        private CheckBox chkRunNow;

        private RadioButton rdoRepair;
        private RadioButton rdoUninstall;
        private bool isAlreadyInstalled = false;

        private int currentStep = 1;
        private string installPath;

        public InstallerForm() {
            this.Text = "Asistente de Instalación de Adobo Reader";
            this.Size = new Size(550, 400);
            this.FormBorderStyle = FormBorderStyle.FixedDialog;
            this.MaximizeBox = false;
            this.StartPosition = FormStartPosition.CenterScreen;
            this.BackColor = Color.FromArgb(240, 240, 240);

            // Detectar instalación existente
            string regInstallPath = null;
            try {
                using (RegistryKey key = Registry.LocalMachine.OpenSubKey(@"Software\Microsoft\Windows\CurrentVersion\Uninstall\AdoboReader")) {
                    if (key != null) {
                        regInstallPath = key.GetValue("InstallLocation") as string;
                    }
                }
            } catch {}

            if (!string.IsNullOrEmpty(regInstallPath)) {
                installPath = regInstallPath;
                isAlreadyInstalled = true;
            } else {
                installPath = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.ProgramFiles), "Adobo");
                isAlreadyInstalled = false;
            }

            InitializeComponents();
            ShowStep();
        }

        private void InitializeComponents() {
            // Banner superior oscuro premium
            bannerPanel = new Panel();
            bannerPanel.Size = new Size(550, 75);
            bannerPanel.Location = new Point(0, 0);
            bannerPanel.BackColor = Color.FromArgb(15, 18, 25);
            this.Controls.Add(bannerPanel);

            bannerLabel = new Label();
            bannerLabel.Text = "Instalación de Adobo Reader";
            bannerLabel.Font = new Font("Segoe UI", 16, FontStyle.Bold);
            bannerLabel.ForeColor = Color.White;
            bannerLabel.Location = new Point(20, 20);
            bannerLabel.AutoSize = true;
            bannerPanel.Controls.Add(bannerLabel);

            // Texto de contenido principal
            contentLabel = new Label();
            contentLabel.Font = new Font("Segoe UI", 10);
            contentLabel.Size = new Size(500, 80);
            contentLabel.Location = new Point(25, 95);
            this.Controls.Add(contentLabel);

            // Inputs del paso 2 (Ruta)
            txtInstallPath = new TextBox();
            txtInstallPath.Size = new Size(360, 25);
            txtInstallPath.Location = new Point(25, 200);
            txtInstallPath.Font = new Font("Segoe UI", 10);
            txtInstallPath.Text = installPath;
            txtInstallPath.Visible = false;
            this.Controls.Add(txtInstallPath);

            btnBrowse = new Button();
            btnBrowse.Text = "Examinar...";
            btnBrowse.Size = new Size(100, 28);
            btnBrowse.Location = new Point(400, 198);
            btnBrowse.Font = new Font("Segoe UI", 9);
            btnBrowse.Click += (s, e) => {
                using (FolderBrowserDialog fbd = new FolderBrowserDialog()) {
                    fbd.SelectedPath = txtInstallPath.Text;
                    if (fbd.ShowDialog() == DialogResult.OK) {
                        txtInstallPath.Text = fbd.SelectedPath;
                    }
                }
            };
            btnBrowse.Visible = false;
            this.Controls.Add(btnBrowse);

            // Checkboxes del paso 3 (Opciones)
            chkDesktop = new CheckBox() { Text = "Crear acceso directo en el Escritorio", Location = new Point(30, 140), Size = new Size(400, 25), Checked = true, Font = new Font("Segoe UI", 9.5f), Visible = false };
            chkStartMenu = new CheckBox() { Text = "Crear acceso directo en el Menú Inicio", Location = new Point(30, 170), Size = new Size(400, 25), Checked = true, Font = new Font("Segoe UI", 9.5f), Visible = false };
            chkAssociate = new CheckBox() { Text = "Asociar archivos .pdf con Adobo Reader", Location = new Point(30, 200), Size = new Size(400, 25), Checked = true, Font = new Font("Segoe UI", 9.5f), Visible = false };
            chkPath = new CheckBox() { Text = "Añadir Adobo al PATH de usuario", Location = new Point(30, 230), Size = new Size(400, 25), Checked = true, Font = new Font("Segoe UI", 9.5f), Visible = false };
            
            this.Controls.Add(chkDesktop);
            this.Controls.Add(chkStartMenu);
            this.Controls.Add(chkAssociate);
            this.Controls.Add(chkPath);

            // Checkbox paso final
            chkRunNow = new CheckBox() { Text = "Ejecutar Adobo Reader ahora", Location = new Point(30, 180), Size = new Size(400, 25), Checked = true, Font = new Font("Segoe UI", 9.5f), Visible = false };
            this.Controls.Add(chkRunNow);

            // RadioButtons para re-instalación/reparación
            rdoRepair = new RadioButton() {
                Text = "Reparar la instalación de Adobo Reader (restaura archivos, accesos directos, registro y PATH)",
                Location = new Point(30, 140),
                Size = new Size(480, 40),
                Checked = true,
                Font = new Font("Segoe UI", 9.5f),
                Visible = false
            };
            rdoUninstall = new RadioButton() {
                Text = "Desinstalar la aplicación por completo de este equipo",
                Location = new Point(30, 190),
                Size = new Size(480, 40),
                Font = new Font("Segoe UI", 9.5f),
                Visible = false
            };
            this.Controls.Add(rdoRepair);
            this.Controls.Add(rdoUninstall);

            // Barra de progreso del paso 4
            progressBar = new ProgressBar();
            progressBar.Size = new Size(480, 25);
            progressBar.Location = new Point(25, 200);
            progressBar.Visible = false;
            this.Controls.Add(progressBar);

            // Botones de navegación
            btnCancel = new Button();
            btnCancel.Text = "Cancelar";
            btnCancel.Size = new Size(85, 30);
            btnCancel.Location = new Point(435, 315);
            btnCancel.Font = new Font("Segoe UI", 9);
            btnCancel.Click += (s, e) => this.Close();
            this.Controls.Add(btnCancel);

            btnNext = new Button();
            btnNext.Text = "Siguiente >";
            btnNext.Size = new Size(85, 30);
            btnNext.Location = new Point(340, 315);
            btnNext.Font = new Font("Segoe UI", 9);
            btnNext.Click += btnNext_Click;
            this.Controls.Add(btnNext);

            btnBack = new Button();
            btnBack.Text = "< Atrás";
            btnBack.Size = new Size(85, 30);
            btnBack.Location = new Point(245, 315);
            btnBack.Font = new Font("Segoe UI", 9);
            btnBack.Click += (s, e) => {
                currentStep--;
                ShowStep();
            };
            this.Controls.Add(btnBack);
        }

        private void ShowStep() {
            // Ocultar todos los inputs por defecto
            txtInstallPath.Visible = false;
            btnBrowse.Visible = false;
            chkDesktop.Visible = false;
            chkStartMenu.Visible = false;
            chkAssociate.Visible = false;
            chkPath.Visible = false;
            progressBar.Visible = false;
            chkRunNow.Visible = false;
            rdoRepair.Visible = false;
            rdoUninstall.Visible = false;

            btnBack.Enabled = (currentStep > 1 && currentStep < 4);
            btnNext.Enabled = true;
            btnCancel.Enabled = (currentStep < 5);

            if (currentStep == 1) {
                if (isAlreadyInstalled) {
                    bannerLabel.Text = "Instalación Detectada";
                    contentLabel.Text = "Se ha detectado una instalación de Adobo Reader en la siguiente carpeta:\n" + installPath + "\n\nPor favor, seleccione qué tarea desea realizar:";
                    rdoRepair.Visible = true;
                    rdoUninstall.Visible = true;
                    rdoRepair.BringToFront();
                    rdoUninstall.BringToFront();
                    btnNext.Text = "Siguiente >";
                } else {
                    bannerLabel.Text = "Bienvenido a Adobo";
                    contentLabel.Text = "Este asistente le guiará a través de la instalación de Adobo Reader en su equipo.\n\n" +
                                         "Adobo Reader es un visor de documentos PDF minimalista y ultra-rápido acelerado por hardware (GPU).\n\n" +
                                         "Presione Siguiente para continuar.";
                    btnNext.Text = "Siguiente >";
                }
            }
            else if (currentStep == 2) {
                bannerLabel.Text = "Carpeta de Destino";
                contentLabel.Text = "Seleccione la carpeta en la que se instalarán los archivos de Adobo Reader.\n\n" +
                                     "Para instalar en la carpeta predeterminada, presione Siguiente. Si desea elegir una carpeta diferente, haga clic en Examinar.";
                txtInstallPath.Visible = true;
                btnBrowse.Visible = true;
                txtInstallPath.BringToFront();
                btnBrowse.BringToFront();
            }
            else if (currentStep == 3) {
                bannerLabel.Text = "Tareas Adicionales";
                contentLabel.Text = "Seleccione las tareas adicionales que desea realizar durante la instalación de Adobo Reader:";
                chkDesktop.Visible = true;
                chkStartMenu.Visible = true;
                chkAssociate.Visible = true;
                chkPath.Visible = true;
                chkDesktop.BringToFront();
                chkStartMenu.BringToFront();
                chkAssociate.BringToFront();
                chkPath.BringToFront();
                btnNext.Text = "Instalar";
            }
            else if (currentStep == 4) {
                bannerLabel.Text = "Instalando...";
                contentLabel.Text = "Espere mientras se realiza la copia de archivos y la configuración del sistema...";
                progressBar.Visible = true;
                btnBack.Enabled = false;
                btnNext.Enabled = false;
                btnCancel.Enabled = false;

                // Iniciamos la instalación
                PerformInstallation();
            }
            else if (currentStep == 5) {
                bannerLabel.Text = "Instalación Completada";
                contentLabel.Text = "Adobo Reader se ha instalado correctamente en su equipo.\n\n" +
                                     "Puede ejecutarlo desde el acceso directo del escritorio o escribiendo 'adobo' en la consola.";
                chkRunNow.Visible = true;
                btnNext.Text = "Finalizar";
                btnNext.Enabled = true;
                btnCancel.Visible = false;
                btnBack.Visible = false;
                btnNext.Location = new Point(435, 315);
            }
        }

        private void btnNext_Click(object sender, EventArgs e) {
            if (currentStep == 1 && isAlreadyInstalled) {
                if (rdoRepair.Checked) {
                    currentStep = 4;
                    ShowStep();
                } else {
                    PerformUninstall();
                }
            }
            else if (currentStep == 3) {
                installPath = txtInstallPath.Text;
                currentStep = 4;
                ShowStep();
            }
            else if (currentStep == 5) {
                if (chkRunNow.Checked && chkRunNow.Visible) {
                    try {
                        System.Diagnostics.Process.Start(Path.Combine(installPath, "adobo.exe"));
                    } catch {}
                }
                this.Close();
            }
            else {
                currentStep++;
                ShowStep();
            }
        }

        private void PerformInstallation() {
            Timer t = new Timer();
            t.Interval = 100;
            int progress = 0;
            t.Tick += (s, ev) => {
                progress += 10;
                if (progress <= 100) {
                    progressBar.Value = progress;
                }

                if (progress == 30) {
                    // Copiar archivos reales
                    try {
                        if (!Directory.Exists(installPath)) {
                            Directory.CreateDirectory(installPath);
                        }

                        // Extraer el ejecutable embebido
                        ExtractResource("ufreader.exe", Path.Combine(installPath, "adobo.exe"));

                        string assetsDir = Path.Combine(installPath, "assets");
                        if (!Directory.Exists(assetsDir)) {
                            Directory.CreateDirectory(assetsDir);
                        }
                        ExtractResource("logo.png", Path.Combine(assetsDir, "logo.png"));
                        ExtractResource("logo.ico", Path.Combine(assetsDir, "logo.ico"));
                        ExtractResource("desinstalar.exe", Path.Combine(installPath, "desinstalar.exe"));
                    }
                    catch (Exception ex) {
                        t.Stop();
                        MessageBox.Show("Error al copiar archivos: " + ex.Message, "Error de Instalación", MessageBoxButtons.OK, MessageBoxIcon.Error);
                        this.Close();
                        return;
                    }
                }
                else if (progress == 60) {
                    // Crear Accesos Directos
                    if (chkDesktop.Checked) {
                        CreateShortcut(
                            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Desktop), "Adobo Reader.lnk"),
                            Path.Combine(installPath, "adobo.exe"),
                            installPath
                        );
                    }
                    if (chkStartMenu.Checked) {
                        string startMenuPrograms = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Programs), "Adobo Reader.lnk");
                        CreateShortcut(startMenuPrograms, Path.Combine(installPath, "adobo.exe"), installPath);
                    }
                }
                else if (progress == 80) {
                    // Registro del sistema para asociaciones
                    if (chkAssociate.Checked) {
                        RegisterFileAssociation();
                    }
                    // PATH
                    if (chkPath.Checked) {
                        AddToPath();
                    }
                    // Registrar en Agregar o quitar programas
                    RegisterUninstaller();
                }
                else if (progress >= 100) {
                    t.Stop();
                    currentStep = 5;
                    ShowStep();
                }
            };
            t.Start();
        }

        private void ExtractResource(string resourceName, string destPath) {
            Assembly assembly = Assembly.GetExecutingAssembly();
            // Buscar el recurso por coincidencia de nombre (los recursos embebidos se nombran con namespace)
            string fullResourceName = null;
            foreach (string name in assembly.GetManifestResourceNames()) {
                if (name.EndsWith(resourceName)) {
                    fullResourceName = name;
                    break;
                }
            }

            if (fullResourceName == null) {
                throw new Exception("Recurso no encontrado: " + resourceName);
            }

            using (Stream stream = assembly.GetManifestResourceStream(fullResourceName)) {
                if (stream == null) throw new Exception("Error al cargar recurso: " + resourceName);
                using (FileStream fs = new FileStream(destPath, FileMode.Create, FileAccess.Write)) {
                    stream.CopyTo(fs);
                }
            }
        }

        private void CreateShortcut(string shortcutPath, string targetPath, string workingDir) {
            try {
                Type shellType = Type.GetTypeFromProgID("WScript.Shell");
                dynamic shell = Activator.CreateInstance(shellType);
                dynamic shortcut = shell.CreateShortcut(shortcutPath);
                shortcut.TargetPath = targetPath;
                shortcut.WorkingDirectory = workingDir;
                shortcut.Description = "Adobo Reader PDF Viewer";
                shortcut.IconLocation = Path.Combine(workingDir, "assets", "logo.ico");
                shortcut.Save();
            }
            catch (Exception ex) {
                Console.WriteLine("Error al crear acceso directo: " + ex.Message);
            }
        }

        private void RegisterFileAssociation() {
            try {
                // Registrar clase AdoboReader.pdf en HKLM (requiere privilegios de administrador)
                using (RegistryKey key = Registry.LocalMachine.CreateSubKey(@"Software\Classes\AdoboReader.pdf")) {
                    key.SetValue("", "Documento PDF (Adobo Reader)");
                    using (RegistryKey commandKey = key.CreateSubKey(@"shell\open\command")) {
                        commandKey.SetValue("", "\"" + Path.Combine(installPath, "adobo.exe") + "\" \"%1\"");
                    }
                    using (RegistryKey iconKey = key.CreateSubKey("DefaultIcon")) {
                        iconKey.SetValue("", "\"" + Path.Combine(installPath, "assets", "logo.ico") + "\"");
                    }
                }

                // Asignar extensión .pdf a la clase
                using (RegistryKey key = Registry.LocalMachine.CreateSubKey(@"Software\Classes\.pdf")) {
                    key.SetValue("", "AdoboReader.pdf");
                }

                // Forzar refresco del Explorador de Windows
                SHChangeNotify(0x08000000, 0, IntPtr.Zero, IntPtr.Zero); // SHCNE_ASSOCCHANGED
            }
            catch (Exception ex) {
                MessageBox.Show("Error al asociar archivos: " + ex.Message, "Advertencia", MessageBoxButtons.OK, MessageBoxIcon.Warning);
            }
        }

        private void AddToPath() {
            try {
                string pathVar = Environment.GetEnvironmentVariable("PATH", EnvironmentVariableTarget.User);
                if (pathVar == null) pathVar = "";
                if (!pathVar.Contains(installPath)) {
                    Environment.SetEnvironmentVariable("PATH", pathVar + ";" + installPath, EnvironmentVariableTarget.User);
                }
            }
            catch (Exception ex) {
                Console.WriteLine("Error al agregar al PATH: " + ex.Message);
            }
        }
        private void RegisterUninstaller() {
            try {
                string uninstallKeyPath = @"Software\Microsoft\Windows\CurrentVersion\Uninstall\AdoboReader";
                using (RegistryKey key = Registry.LocalMachine.CreateSubKey(uninstallKeyPath)) {
                    key.SetValue("DisplayName", "Adobo Reader");
                    key.SetValue("UninstallString", "\"" + Path.Combine(installPath, "desinstalar.exe") + "\"");
                    key.SetValue("DisplayIcon", "\"" + Path.Combine(installPath, "assets", "logo.ico") + "\"");
                    key.SetValue("Publisher", "Adobo Team");
                    key.SetValue("DisplayVersion", "1.0.0");
                    key.SetValue("InstallLocation", installPath);
                    key.SetValue("EstimatedSize", 32000); // ~32 MB
                    key.SetValue("NoModify", 1);
                    key.SetValue("NoRepair", 1);
                }
            }
            catch (Exception ex) {
                Console.WriteLine("Error al registrar el desinstalador: " + ex.Message);
            }
        }

        private void PerformUninstall() {
            try {
                // 1. Eliminar Accesos Directos
                string desktopLnk = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Desktop), "Adobo Reader.lnk");
                if (File.Exists(desktopLnk)) File.Delete(desktopLnk);

                string startMenuLnk = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.Programs), "Adobo Reader.lnk");
                if (File.Exists(startMenuLnk)) File.Delete(startMenuLnk);

                // 2. Eliminar Asociaciones del Registro
                using (RegistryKey pdfKey = Registry.LocalMachine.OpenSubKey(@"Software\Classes\.pdf", true)) {
                    if (pdfKey != null && "AdoboReader.pdf".Equals(pdfKey.GetValue(""))) {
                        pdfKey.SetValue("", "");
                    }
                }
                try {
                    Registry.LocalMachine.DeleteSubKeyTree(@"Software\Classes\AdoboReader.pdf", false);
                } catch {}

                // Forzar refresco
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

                // 5. Borrado diferido mediante archivo BAT
                string tempBat = Path.Combine(Path.GetTempPath(), "cleanup_adobo.bat");
                using (StreamWriter sw = new StreamWriter(tempBat)) {
                    sw.WriteLine("@echo off");
                    sw.WriteLine(":loop");
                    sw.WriteLine("taskkill /f /im adobo.exe >nul 2>&1");
                    sw.WriteLine("del \"" + Path.Combine(installPath, "adobo.exe") + "\" >nul 2>&1");
                    sw.WriteLine("del \"" + Path.Combine(installPath, "desinstalar.exe") + "\" >nul 2>&1");
                    sw.WriteLine("if exist \"" + Path.Combine(installPath, "adobo.exe") + "\" goto loop");
                    sw.WriteLine("rd /s /q \"" + installPath + "\" >nul 2>&1");
                    sw.WriteLine("del \"%~f0\" >nul 2>&1");
                }

                System.Diagnostics.ProcessStartInfo psi = new System.Diagnostics.ProcessStartInfo(tempBat);
                psi.WindowStyle = System.Diagnostics.ProcessWindowStyle.Hidden;
                psi.CreateNoWindow = true;
                psi.UseShellExecute = true;
                System.Diagnostics.Process.Start(psi);

                // Ir a la pantalla de completado con texto de desinstalación
                currentStep = 5;
                ShowStep();
                bannerLabel.Text = "Desinstalación Completada";
                contentLabel.Text = "Adobo Reader se ha desinstalado correctamente de su equipo.";
                chkRunNow.Visible = false;
                btnNext.Text = "Cerrar";
            }
            catch (Exception ex) {
                MessageBox.Show("Error durante la desinstalación: " + ex.Message, "Error", MessageBoxButtons.OK, MessageBoxIcon.Error);
                this.Close();
            }
        }

        [DllImport("shell32.dll", CharSet = CharSet.Auto, SetLastError = true)]
        public static extern void SHChangeNotify(int wEventId, int uFlags, IntPtr dwItem1, IntPtr dwItem2);

        [STAThread]
        public static void Main() {
            Application.EnableVisualStyles();
            Application.SetCompatibleTextRenderingDefault(false);
            Application.Run(new InstallerForm());
        }
    }
}
