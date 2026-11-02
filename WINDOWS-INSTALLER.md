# Тестовый установщик Windows 11 x64

Комплект не содержит готового EXE: его создаёт Windows runner GitHub Actions.
Сборка и установщик в среде создания комплекта не запускались.

Распакуйте комплект в корень проекта, сохраняя структуру, включая .github.
Добавьте файлы в git, закоммитьте, отправьте в master. Workflow с ручным
запуском должен присутствовать в основной ветке репозитория.
GitHub -> Actions -> Windows installer -> Run workflow -> master.
После успешной сборки скачайте artifact VideoLoader-windows-x64-installer.
Распакуйте artifact в Windows: внутри setup.exe и SHA256.
Workflow не создаёт GitHub Release и не меняет теги. Проверьте квоту Actions.

Перед сборкой выполняются тесты. Используется x86_64-pc-windows-msvc
со статическим CRT для целевого приложения. Проверьте на чистой Windows.
ARM64 пока исключён. Зависимости не включены в дистрибутив: их ставит
отдельный помощник через WinGet с подтверждением.

Проверка хеша в PowerShell:
    Get-FileHash .\VideoLoader-0.12.0-windows-x64-setup.exe -Algorithm SHA256
Сравните с соответствующим .sha256 в artifact.
