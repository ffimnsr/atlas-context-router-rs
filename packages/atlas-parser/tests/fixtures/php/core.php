<?php
namespace Demo\App;

use Demo\Support\Helper;

#[Service]
class Runner {
    #[Trace]
    public function run() {
        helper();
    }

    private function helper() {}
}

trait UsesLog {}
interface RunnerContract {}

function helper() {}
