(Get-Content src\welcome.rs) -replace 'WindowEvent::RedrawRequested => \{', 'WindowEvent::RedrawRequested => { self.draw(); } _ => {} } } pub fn draw(&mut self) {' | Set-Content src\welcome.rs
