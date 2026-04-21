#include <vector>

namespace demo {

template <typename T>
class Box {};

class Runner {
public:
    void helper() {}
    void run() {
        helper();
    }
};

void free_fn() {}

}
