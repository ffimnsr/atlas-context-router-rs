using System.Text;

namespace Demo.App;

[Service]
class Runner
{
    [Trace]
    void Run()
    {
        Helper();
    }

    void Helper() { }
}

interface IRunner { }
struct Bag { }
enum Mode { On }
