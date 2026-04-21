package demo.app;

import java.util.List;

@Service
class Main {
    @Trace
    void run() {
        helper();
    }

    void helper() {}
}

interface Api {
    void ping();
}

enum Mode {
    ON,
}
