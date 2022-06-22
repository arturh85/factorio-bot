## Architecture

***work in progress***

```mermaid
graph TD
    A[Christmas] -->|Get money| B(Go shopping)
    B --> C{Let me think}
    C -->|One| D[Laptop]
    C -->|Two| E[iPhone]
    C -->|Three| F[fa:fa-car Car]
```

```mermaid
sequenceDiagram
    participant F as Frontend
    participant C as Tauri Core

    F-)+C: IPC request
    note over C: Perform computation, write to file system, etc.
    C-)-F: Response
```

